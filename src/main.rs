#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! PS2 Audio Extractor: finds the audio in a PS2 game (disc image, extracted disc folder
//! or single file) and saves it as WAV files.
//!
//! The window is frameless. The page draws its own title bar and sends window actions
//! (drag, minimize, maximize, close) over IPC.

mod codecs;
mod disc;
mod extract;
mod formats;
mod scan;
mod track;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::Instant;

use serde_json::{Value, json};
use tao::dpi::LogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::window::{Theme, WindowBuilder};
use wry::http::{Request, Response, header};
use wry::{DragDropEvent, WebContext, WebViewBuilder};

use disc::Game;
use extract::Loops;
use track::Track;

const INDEX_HTML: &str = include_str!("../ui/index.html");
const APP_CSS: &str = include_str!("../ui/app.css");
const APP_JS: &str = include_str!("../ui/app.js");
/// Previews stop after this long, to keep them quick to decode.
const PREVIEW_SECONDS: u32 = 240;

enum UserEvent {
    Js(String),
    Window(WindowCmd),
}

enum WindowCmd {
    Drag,
    Minimize,
    ToggleMaximize,
    Close,
}

struct State {
    proxy: Mutex<Option<EventLoopProxy<UserEvent>>>,
    game: Mutex<Option<Arc<Game>>>,
    tracks: Mutex<Arc<Vec<Track>>>,
    busy: AtomicBool,
    cancel: AtomicBool,
    /// The last preview, so the audio element's range requests don't decode it again.
    preview: Mutex<Option<(usize, Arc<Vec<u8>>)>>,
}

static STATE: OnceLock<State> = OnceLock::new();

fn state() -> &'static State {
    STATE.get_or_init(|| State {
        proxy: Mutex::new(None),
        game: Mutex::new(None),
        tracks: Mutex::new(Arc::new(Vec::new())),
        busy: AtomicBool::new(false),
        cancel: AtomicBool::new(false),
        preview: Mutex::new(None),
    })
}

fn send(ev: UserEvent) {
    if let Some(p) = state().proxy.lock().unwrap().as_ref() {
        let _ = p.send_event(ev);
    }
}

fn send_js(msg: Value) {
    send(UserEvent::Js(format!("window.__rx({msg});")));
}

fn event(data: Value) {
    send_js(json!({ "kind": "event", "data": data }));
}

fn reply(id: u64, result: Result<Value, String>) {
    match result {
        Ok(data) => send_js(json!({ "kind": "reply", "id": id, "ok": true, "data": data })),
        Err(error) => send_js(json!({ "kind": "reply", "id": id, "ok": false, "error": error })),
    }
}

// ---------------------------------------------------------------------------------------
// Commands from the page
// ---------------------------------------------------------------------------------------

fn handle_ipc(body: &str) {
    let Ok(msg) = serde_json::from_str::<Value>(body) else { return };
    let id = msg["id"].as_u64().unwrap_or(0);
    let cmd = msg["cmd"].as_str().unwrap_or("").to_string();
    let args = msg["args"].clone();

    // Window actions must happen right away (a drag only works while the button is down).
    let window_cmd = match cmd.as_str() {
        "win_drag" => Some(WindowCmd::Drag),
        "win_minimize" => Some(WindowCmd::Minimize),
        "win_maximize" => Some(WindowCmd::ToggleMaximize),
        "win_close" => Some(WindowCmd::Close),
        _ => None,
    };
    if let Some(w) = window_cmd {
        send(UserEvent::Window(w));
        return;
    }

    thread::spawn(move || {
        let result = match cmd.as_str() {
            "init" => Ok(json!({ "version": env!("CARGO_PKG_VERSION") })),
            "pick_source" => pick_source(args["folder"].as_bool().unwrap_or(false)),
            "open_source" => open_source(PathBuf::from(args["path"].as_str().unwrap_or(""))),
            "scan" => start_scan(&args),
            "pick_output" => Ok(rfd::FileDialog::new()
                .set_title("Save the audio in…")
                .pick_folder()
                .map(|p| json!(p.to_string_lossy()))
                .unwrap_or(Value::Null)),
            "extract" => start_extract(&args),
            "cancel" => {
                state().cancel.store(true, Ordering::Relaxed);
                Ok(Value::Null)
            }
            "open_path" => open::that_detached(args["path"].as_str().unwrap_or("")).map(|_| Value::Null).map_err(|e| e.to_string()),
            other => Err(format!("unknown command {other}")),
        };
        reply(id, result);
    });
}

fn pick_source(folder: bool) -> Result<Value, String> {
    let dialog = rfd::FileDialog::new();
    let picked = if folder {
        dialog.set_title("Open an extracted PS2 disc folder").pick_folder()
    } else {
        dialog
            .set_title("Open a PS2 disc image or game file")
            .add_filter("Disc images", &["iso", "bin", "img"])
            .add_filter("All files", &["*"])
            .pick_file()
    };
    match picked {
        Some(p) => open_source(p),
        None => Ok(Value::Null),
    }
}

fn open_source(path: PathBuf) -> Result<Value, String> {
    if state().busy.load(Ordering::SeqCst) {
        return Err("Wait for the current scan or extraction to finish".into());
    }
    let game = Game::open(&path)?;
    let size: u64 = game.entries.iter().map(|e| e.size).sum();
    let info = json!({
        "path": path.to_string_lossy(),
        "title": game.name,
        "serial": game.serial,
        "files": game.entries.len(),
        "size": size,
        "is_folder": path.is_dir(),
        // A sensible default for where the WAVs go: next to the source.
        "default_output": path.parent().unwrap_or(&path).join("PS2 Audio").to_string_lossy(),
    });
    *state().game.lock().unwrap() = Some(Arc::new(game));
    *state().tracks.lock().unwrap() = Arc::new(Vec::new());
    *state().preview.lock().unwrap() = None;
    Ok(info)
}

/// Claims the one background job slot.
fn begin_job() -> Result<(), String> {
    if state().busy.swap(true, Ordering::SeqCst) {
        return Err("Already working on something".into());
    }
    state().cancel.store(false, Ordering::SeqCst);
    Ok(())
}

fn start_scan(args: &Value) -> Result<Value, String> {
    let game = state().game.lock().unwrap().clone().ok_or("Open a game first")?;
    let opts = scan::Options {
        headerless: args["headerless"].as_bool().unwrap_or(true),
        headerless_rate: args["headerless_rate"].as_u64().map(|r| r as u32).filter(|r| (4000..=96000).contains(r)).unwrap_or(22050),
    };
    // Scan the files the game was opened with (not ones unpacked by an earlier scan).
    let game = Arc::new(game.with_entries(game.entries.iter().filter(|e| !e.path.contains(" [")).cloned().collect()));
    begin_job()?;
    thread::spawn(move || {
        let total: u64 = game.entries.iter().map(|e| e.size).sum();
        let started = Instant::now();
        let mut last = Instant::now();
        let result = scan::scan(&game, opts, &state().cancel, &mut |done, file| {
            if last.elapsed().as_millis() >= 100 {
                last = Instant::now();
                event(json!({ "name": "scan_progress", "done": done, "total": total, "file": file }));
            }
        });
        state().busy.store(false, Ordering::SeqCst);
        match result {
            Ok(res) => {
                let files: Vec<&str> = res.entries.iter().map(|e| e.path.as_str()).collect();
                event(json!({
                    "name": "scan_done",
                    "tracks": res.tracks,
                    "files": files,
                    "seconds": started.elapsed().as_secs_f64(),
                }));
                // Keep the in-memory files the scan made: tracks read from them.
                *state().game.lock().unwrap() = Some(Arc::new(game.with_entries(res.entries)));
                *state().tracks.lock().unwrap() = Arc::new(res.tracks);
                *state().preview.lock().unwrap() = None;
            }
            Err(e) => event(json!({ "name": "scan_done", "error": e, "cancelled": e == "cancelled" })),
        }
    });
    Ok(Value::Null)
}

fn start_extract(args: &Value) -> Result<Value, String> {
    let game = state().game.lock().unwrap().clone().ok_or("Open a game first")?;
    let out = PathBuf::from(args["output"].as_str().filter(|s| !s.is_empty()).ok_or("Choose where to save the audio")?);
    let all = state().tracks.lock().unwrap().clone();
    let tracks: Vec<Track> = match args["ids"].as_array() {
        Some(ids) => {
            let ids: std::collections::HashSet<u64> = ids.iter().filter_map(Value::as_u64).collect();
            all.iter().filter(|t| ids.contains(&(t.id as u64))).cloned().collect()
        }
        None => all.to_vec(),
    };
    if tracks.is_empty() {
        return Err("Nothing to extract".into());
    }
    let loops = if args["loops"].as_str() == Some("twice") { Loops::Twice } else { Loops::Once };
    begin_job()?;
    thread::spawn(move || {
        let mut last = Instant::now();
        let result = extract::extract(&game, &tracks, &out, loops, &state().cancel, &mut |done, total, file| {
            if last.elapsed().as_millis() >= 80 || done == total {
                last = Instant::now();
                event(json!({ "name": "extract_progress", "done": done, "total": total, "file": file }));
            }
        });
        state().busy.store(false, Ordering::SeqCst);
        match result {
            Ok(s) => event(json!({
                "name": "extract_done",
                "folder": s.folder.to_string_lossy(),
                "written": s.written,
                "skipped": s.skipped,
                "failed": s.failed,
            })),
            Err(e) => event(json!({ "name": "extract_done", "error": e, "cancelled": e == "cancelled" })),
        }
    });
    Ok(Value::Null)
}

// ---------------------------------------------------------------------------------------
// app:// (the page, and track previews)
// ---------------------------------------------------------------------------------------

fn respond(req: Request<Vec<u8>>) -> Response<Vec<u8>> {
    let path = req.uri().path();
    let text = |body: &str, mime: &str| {
        Response::builder()
            .header(header::CONTENT_TYPE, mime)
            .header(header::CACHE_CONTROL, "no-cache")
            .body(body.as_bytes().to_vec())
            .unwrap()
    };
    match path {
        "/" | "/index.html" => text(INDEX_HTML, "text/html; charset=utf-8"),
        "/app.css" => text(APP_CSS, "text/css; charset=utf-8"),
        "/app.js" => text(APP_JS, "text/javascript; charset=utf-8"),
        "/preview" => preview(&req),
        _ => status(404, "not found"),
    }
}

fn status(code: u16, msg: &str) -> Response<Vec<u8>> {
    Response::builder().status(code).body(msg.as_bytes().to_vec()).unwrap()
}

/// A track decoded to WAV for the page's audio player, with Range support for seeking.
fn preview(req: &Request<Vec<u8>>) -> Response<Vec<u8>> {
    let id = req.uri().query().unwrap_or("").split('&').find_map(|kv| kv.strip_prefix("id=")).and_then(|v| v.parse::<usize>().ok());
    let Some(id) = id else { return status(400, "missing id") };
    let cached = state().preview.lock().unwrap().as_ref().filter(|(i, _)| *i == id).map(|(_, b)| b.clone());
    let bytes = match cached {
        Some(b) => b,
        None => {
            let (Some(game), tracks) = (state().game.lock().unwrap().clone(), state().tracks.lock().unwrap().clone()) else {
                return status(404, "no game");
            };
            let Some(track) = tracks.get(id) else { return status(404, "no such track") };
            match extract::wav_bytes(&game, track, PREVIEW_SECONDS) {
                Ok(b) => {
                    let b = Arc::new(b);
                    *state().preview.lock().unwrap() = Some((id, b.clone()));
                    b
                }
                Err(e) => return status(500, &e.to_string()),
            }
        }
    };
    let len = bytes.len() as u64;
    let range = req.headers().get(header::RANGE).and_then(|v| v.to_str().ok()).and_then(|r| {
        let (a, b) = r.strip_prefix("bytes=")?.split_once('-')?;
        let start: u64 = a.parse().ok()?;
        let end: u64 = if b.is_empty() { len - 1 } else { b.parse().ok()? };
        (start <= end && end < len).then_some((start, end))
    });
    let builder = Response::builder().header(header::CONTENT_TYPE, "audio/wav").header(header::ACCEPT_RANGES, "bytes");
    match range {
        Some((a, b)) => builder
            .status(206)
            .header(header::CONTENT_RANGE, format!("bytes {a}-{b}/{len}"))
            .body(bytes[a as usize..=b as usize].to_vec())
            .unwrap(),
        None => builder.body(bytes.to_vec()).unwrap(),
    }
}

// ---------------------------------------------------------------------------------------
// Window
// ---------------------------------------------------------------------------------------

fn main() -> wry::Result<()> {
    // Command line: `ps2audioextractor <game> <output folder> [--loops-twice] [--no-headerless]
    // [--headerless-rate N]` scans and extracts without the window (batch use, testing).
    let args: Vec<String> = std::env::args().skip(1).collect();
    let positional: Vec<&String> = args.iter().filter(|a| !a.starts_with("--") && a.parse::<u32>().is_err()).collect();
    if positional.len() == 2 {
        let mut opts = scan::Options::default();
        if args.iter().any(|a| a == "--no-headerless") {
            opts.headerless = false;
        }
        if let Some(i) = args.iter().position(|a| a == "--headerless-rate") {
            opts.headerless_rate = args.get(i + 1).and_then(|v| v.parse().ok()).unwrap_or(22050);
        }
        let loops = if args.iter().any(|a| a == "--loops-twice") { Loops::Twice } else { Loops::Once };
        std::process::exit(run_cli(positional[0], positional[1], opts, loops));
    }

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    *state().proxy.lock().unwrap() = Some(event_loop.create_proxy());

    let builder = WindowBuilder::new()
        .with_title("PS2 Audio Extractor")
        // frameless, the page draws its own title bar
        .with_decorations(false)
        .with_inner_size(LogicalSize::new(1120.0, 760.0))
        .with_min_inner_size(LogicalSize::new(760.0, 520.0))
        .with_theme(Some(Theme::Dark))
        .with_background_color((0, 0, 0, 255));
    #[cfg(windows)]
    let builder = {
        use tao::platform::windows::WindowBuilderExtWindows;
        builder.with_undecorated_shadow(true)
    };
    let window = builder.build(&event_loop).expect("failed to create window");

    // Keep WebView2's data out of the program's folder.
    let data_dir = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("ps2audioextractor");
    let mut context = WebContext::new(Some(data_dir));
    let webview = WebViewBuilder::new_with_web_context(&mut context)
        .with_url("app://localhost/")
        .with_background_color((0, 0, 0, 255))
        .with_devtools(cfg!(debug_assertions))
        .with_asynchronous_custom_protocol("app".into(), |_id, request, responder| {
            thread::spawn(move || responder.respond(respond(request)));
        })
        .with_ipc_handler(|req| handle_ipc(req.body()))
        .with_drag_drop_handler(|ev| {
            match ev {
                DragDropEvent::Enter { .. } => event(json!({ "name": "drag", "over": true })),
                DragDropEvent::Leave => event(json!({ "name": "drag", "over": false })),
                DragDropEvent::Drop { paths, .. } => {
                    event(json!({ "name": "drag", "over": false }));
                    if let Some(p) = paths.first() {
                        event(json!({ "name": "dropped", "path": p.to_string_lossy() }));
                    }
                }
                _ => {}
            }
            true // never let the page navigate to a dropped file
        })
        .with_navigation_handler(|url| url.starts_with("app://") || url.starts_with("http://app.localhost"))
        .build(&window)?;

    let mut maximized = window.is_maximized();
    event_loop.run(move |ev, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match ev {
            Event::UserEvent(UserEvent::Js(js)) => {
                let _ = webview.evaluate_script(&js);
            }
            Event::UserEvent(UserEvent::Window(cmd)) => match cmd {
                WindowCmd::Drag => {
                    let _ = window.drag_window();
                }
                WindowCmd::Minimize => window.set_minimized(true),
                WindowCmd::ToggleMaximize => window.set_maximized(!window.is_maximized()),
                WindowCmd::Close => *control_flow = ControlFlow::Exit,
            },
            Event::WindowEvent { event: WindowEvent::Resized(_), .. } => {
                if window.is_maximized() != maximized {
                    maximized = window.is_maximized();
                    event(json!({ "name": "window", "maximized": maximized }));
                }
            }
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => *control_flow = ControlFlow::Exit,
            _ => {}
        }
    });
}

/// Scans and extracts from the command line; returns the exit code.
fn run_cli(input: &str, output: &str, opts: scan::Options, loops: Loops) -> i32 {
    let game = match Game::open(std::path::Path::new(input)) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    println!("{} ({} files)", game.title, game.entries.len());
    let cancel = AtomicBool::new(false);
    let (tracks, game) = match scan::scan(&game, opts, &cancel, &mut |_, _| {}) {
        Ok(res) => (res.tracks, game.with_entries(res.entries)),
        Err(e) => {
            eprintln!("scan failed: {e}");
            return 1;
        }
    };
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for t in &tracks {
        *counts.entry(t.format).or_default() += 1;
        println!(
            "{:5} {:6} {}ch {:5}Hz {:8.2}s  {}  <- {} @0x{:X}{}{}",
            t.id, t.format, t.channels, t.sample_rate, t.duration(), t.path, game.entries[t.entry].path, t.offset,
            match (t.loop_start, t.loop_end) {
                (Some(a), Some(b)) => format!("  loop {a}-{b}"),
                _ => String::new(),
            },
            t.note.as_deref().map(|n| format!("  [{n}]")).unwrap_or_default()
        );
    }
    println!("found {} tracks: {counts:?}", tracks.len());
    match extract::extract(&game, &tracks, std::path::Path::new(output), loops, &cancel, &mut |_, _, _| {}) {
        Ok(s) => {
            println!("wrote {} files to {}", s.written, s.folder.display());
            for m in s.skipped.iter().chain(&s.failed) {
                println!("  {m}");
            }
            if s.failed.is_empty() { 0 } else { 2 }
        }
        Err(e) => {
            eprintln!("extract failed: {e}");
            1
        }
    }
}
