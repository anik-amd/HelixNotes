mod ai;
mod asset_scope;
mod backup;
mod commands;
mod history;
mod image_proxy;
mod search;
mod state;
mod sync;
mod types;
mod vault;

use state::AppState;
#[allow(unused_imports)]
use tauri::{Emitter, Manager};
use tauri_plugin_fs::FsExt;

#[cfg(desktop)]
use tauri::{
    image::Image,
    menu::{MenuBuilder, MenuItemBuilder},
    tray::TrayIconBuilder,
};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Work around blank window from WebKitGTK DMABUF/GBM allocation failures on some Linux GPUs.
    #[cfg(target_os = "linux")]
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }

    // A dependency may already have installed a rustls provider (for example through
    // reqwest). Keep that provider instead of aborting the app during startup.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let config = commands::load_app_config();
    #[cfg(desktop)]
    let show_tray = config.show_tray_icon;
    let app_state = AppState::new(config);

    // Inject the compile-time platform so the frontend never sniffs the (sometimes
    // mobile-looking) WebKitGTK user-agent. (#63)
    let platform_init = format!(
        "window.__HELIX_PLATFORM__={{mobile:{},android:{},ios:{}}};",
        cfg!(mobile),
        cfg!(target_os = "android"),
        cfg!(target_os = "ios"),
    );

    let mut builder = tauri::Builder::default()
        .plugin(
            tauri::plugin::Builder::<tauri::Wry>::new("helix-platform")
                .js_init_script(platform_init)
                .build(),
        )
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_opener::init())
        .on_webview_event(|webview, event| {
            if let tauri::WebviewEvent::DragDrop(tauri::DragDropEvent::Drop { paths, .. }) = event {
                let scope = webview.fs_scope();
                for path in paths {
                    if path.is_file() {
                        let _ = scope.allow_file(path);
                    }
                }
            }
        })
        .on_page_load(|webview, payload| {
            #[cfg(target_os = "macos")]
            if webview.label() == "main"
                && matches!(payload.event(), tauri::webview::PageLoadEvent::Finished)
            {
                queue_macos_traffic_light_fix(&webview.window());
            }
            #[cfg(not(target_os = "macos"))]
            let _ = (webview, payload);
        })
        .setup(move |app| {
            #[cfg(target_os = "ios")]
            app.handle().plugin(tauri_plugin_ios_vault_access::init())?;

            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }

            // On mobile, set config dir from Tauri's path resolver, then reload config
            #[cfg(mobile)]
            {
                if let Ok(config_dir) = app.path().config_dir() {
                    commands::set_mobile_config_dir(config_dir);
                } else if let Ok(data_dir) = app.path().data_dir() {
                    commands::set_mobile_config_dir(data_dir);
                }
                // Reload config now that the mobile config dir is available
                let reloaded = commands::load_app_config();
                let _ = app.state::<AppState>().config.lock().map(|mut cfg| {
                    *cfg = reloaded;
                });
            }

            let active_vault = app
                .state::<AppState>()
                .config
                .lock()
                .ok()
                .and_then(|config| config.active_vault.clone());
            if let Some(vault_path) = active_vault {
                if let Err(error) =
                    asset_scope::allow_vault_assets(app.handle(), std::path::Path::new(&vault_path))
                {
                    log::warn!("Failed to restore the vault asset scope: {error}");
                }
            }

            #[cfg(desktop)]
            if show_tray {
                setup_tray(app)?;
            }

            // Check CLI args for a .md file path on initial launch
            #[cfg(desktop)]
            {
                let file_arg = std::env::args().skip(1).find(|a| {
                    let a = a.trim();
                    !a.starts_with('-') && a.ends_with(".md")
                });
                if let Some(path) = file_arg {
                    let resolved = if std::path::Path::new(&path).is_absolute() {
                        std::path::PathBuf::from(&path)
                    } else {
                        std::env::current_dir().unwrap_or_default().join(&path)
                    };
                    if let Some(resolved_str) = resolved.to_str() {
                        if resolved.is_file() {
                            let _ = app.fs_scope().allow_file(&resolved);
                            if let Some(parent) = resolved.parent() {
                                let _ = app.fs_scope().allow_directory(parent, true);
                            }
                            let _ =
                                asset_scope::allow_external_note_assets(app.handle(), &resolved);
                        }
                        let app_state = app.state::<AppState>();
                        if let Err(error) = app_state.enqueue_open_file(resolved_str.to_string()) {
                            log::error!("Could not queue startup file: {error}");
                        }
                    }
                }
            }

            Ok(())
        })
        .manage(app_state)
        .invoke_handler(tauri::generate_handler![
            commands::open_vault,
            commands::choose_external_vault,
            commands::restore_external_vault,
            commands::remove_vault,
            commands::get_app_config,
            #[cfg(desktop)]
            commands::complete_window_close,
            commands::set_theme,
            commands::set_system_themes,
            commands::set_accent_color,
            commands::save_custom_theme,
            commands::delete_custom_theme,
            commands::export_custom_theme,
            commands::import_custom_themes,
            commands::set_font_size,
            commands::set_scroll_to_change_font_size,
            commands::set_toolbar_layout,
            commands::set_font_family,
            commands::set_line_height,
            commands::set_ui_scale,
            commands::set_content_width,
            commands::get_notebooks,
            commands::count_root_notes,
            commands::create_notebook,
            commands::rename_notebook,
            commands::delete_notebook,
            commands::move_notebook,
            commands::get_notes,
            commands::read_note,
            commands::save_note,
            commands::import_note,
            commands::create_note,
            commands::duplicate_note,
            commands::create_daily_note,
            commands::rename_note,
            commands::delete_note,
            commands::move_note,
            commands::get_all_tags,
            commands::get_all_note_titles,
            commands::get_note_switcher_titles,
            commands::get_graph_data,
            commands::get_tasks,
            commands::set_task_done,
            commands::set_task_priority,
            commands::set_task_due,
            commands::search_notes,
            commands::reindex,
            commands::get_trash,
            commands::restore_note,
            commands::restore_notebook,
            commands::permanent_delete,
            commands::empty_trash,
            commands::load_vault_state,
            commands::save_vault_state,
            commands::copy_text_to_clipboard,
            commands::read_clipboard_contents,
            commands::read_clipboard_image,
            commands::copy_image_to_clipboard,
            commands::save_image,
            commands::save_attachment,
            commands::get_notebook_icons,
            commands::set_notebook_icon,
            commands::import_notebook_icon,
            commands::get_tag_styles,
            commands::set_tag_style,
            commands::import_tag_icon,
            commands::set_general_settings,
            commands::get_quick_access,
            commands::add_quick_access,
            commands::remove_quick_access,
            commands::reorder_quick_access,
            commands::get_vault_stats,
            commands::find_orphaned_attachments,
            commands::trash_orphaned_attachments,
            commands::import_obsidian,
            commands::open_file,
            commands::reveal_file,
            commands::open_url,
            commands::copy_file_to,
            commands::write_bytes_to,
            commands::copy_png_to_clipboard,
            commands::create_backup,
            commands::list_backups,
            commands::restore_backup,
            commands::delete_backup,
            commands::set_backup_settings,
            commands::get_note_versions,
            commands::get_note_version_content,
            commands::create_version,
            commands::set_ai_settings,
            commands::test_ai_connection,
            commands::ai_ask,
            commands::set_sync_settings,
            commands::test_sync_connection,
            commands::sync_now,
            commands::get_install_type,
            commands::is_mobile_platform,
            commands::get_pending_open_file,
            commands::ack_pending_open_file,
            #[cfg(desktop)]
            commands::cancel_window_close,
        ])
        .register_asynchronous_uri_scheme_protocol("imgproxy", |_ctx, request, responder| {
            let path = request.uri().path().to_string();
            std::thread::spawn(move || {
                let encoded = path.strip_prefix('/').unwrap_or(&path);
                let external_url = percent_decode(encoded);
                let response = match image_proxy::fetch(&external_url) {
                    Ok(image) => tauri::http::Response::builder()
                        .status(200)
                        .header("Content-Type", image.content_type)
                        .header("Access-Control-Allow-Origin", "*")
                        .header("X-Content-Type-Options", "nosniff")
                        .body(image.body),
                    Err(error) => {
                        log::warn!("Blocked external image request: {}", error.message());
                        tauri::http::Response::builder()
                            .status(error.status())
                            .body(Vec::new())
                    }
                };
                responder.respond(response.expect("valid image proxy response"));
            });
        });

    #[cfg(desktop)]
    {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, args, cwd| {
            // Always show/focus the main window
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }

            // Extract .md file path from args (args[0] is the binary)
            let file_path = args.iter().skip(1).find(|arg| {
                let a = arg.trim();
                !a.starts_with('-') && a.ends_with(".md")
            });

            if let Some(path) = file_path {
                let resolved = if std::path::Path::new(path.as_str()).is_absolute() {
                    std::path::PathBuf::from(path)
                } else {
                    std::path::Path::new(&cwd).join(path)
                };
                if let Some(resolved_str) = resolved.to_str() {
                    if resolved.is_file() {
                        let _ = app.fs_scope().allow_file(&resolved);
                        if let Some(parent) = resolved.parent() {
                            let _ = app.fs_scope().allow_directory(parent, true);
                        }
                        let _ = asset_scope::allow_external_note_assets(app, &resolved);
                    }
                    if let Err(error) = app
                        .state::<AppState>()
                        .enqueue_open_file(resolved_str.to_string())
                    {
                        log::error!("Could not queue open file: {error}");
                    } else {
                        let _ = app.emit("open-file-pending", ());
                    }
                }
            }
        }));

        builder = builder.plugin(tauri_plugin_updater::Builder::new().build());
        let window_state_builder = tauri_plugin_window_state::Builder::default();
        #[cfg(target_os = "linux")]
        let window_state_builder = window_state_builder.with_state_flags(
            tauri_plugin_window_state::StateFlags::SIZE
                | tauri_plugin_window_state::StateFlags::POSITION
                | tauri_plugin_window_state::StateFlags::MAXIMIZED
                | tauri_plugin_window_state::StateFlags::DECORATIONS
                | tauri_plugin_window_state::StateFlags::FULLSCREEN,
        );
        builder = builder.plugin(window_state_builder.build());

        builder = builder.on_window_event(move |window, event| {
            #[cfg(target_os = "macos")]
            if window.label() == "main"
                && matches!(
                    event,
                    tauri::WindowEvent::Resized(_)
                        | tauri::WindowEvent::Focused(true)
                        | tauri::WindowEvent::ScaleFactorChanged { .. }
                        | tauri::WindowEvent::ThemeChanged(_)
                )
            {
                // AppKit can restore the default button Y position after window-state
                // changes. Queue this pass so it runs after AppKit finishes its layout.
                queue_macos_traffic_light_fix(window);
            }

            // Closing the main window also requests a save-aware close of note windows.
            if matches!(event, tauri::WindowEvent::Destroyed) && window.label() == "main" {
                for (label, win) in window.app_handle().webview_windows() {
                    if label.starts_with("note-") {
                        let _ = win.close();
                    }
                }
            }
        });
    }

    let app = builder
        .build(tauri::generate_context!())
        .expect("error while building tauri application");
    app.run(|app, event| {
        #[cfg(desktop)]
        if let tauri::RunEvent::ExitRequested {
            code: Some(_), api, ..
        } = event
        {
            let windows = app.webview_windows();
            if !windows.is_empty() {
                // Menu/tray Quit must give each editor the same save opportunity
                // as a window close. The app exits once the last window closes.
                api.prevent_exit();
                app.state::<AppState>()
                    .quitting
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                for window in windows.values() {
                    if let Err(error) = window.close() {
                        log::error!("Failed to close window during quit: {error}");
                    }
                }
            }
        }
        #[cfg(mobile)]
        let _ = (app, event);
    });
}

#[cfg(desktop)]
fn setup_tray(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let show = MenuItemBuilder::with_id("show", "Show HelixNotes").build(app)?;
    let quit = MenuItemBuilder::with_id("quit", "Quit").build(app)?;
    let menu = MenuBuilder::new(app).items(&[&show, &quit]).build()?;

    let _tray = TrayIconBuilder::with_id("helixnotes")
        .icon(
            Image::from_path("icons/32x32.png")
                .unwrap_or_else(|_| app.default_window_icon().cloned().unwrap()),
        )
        .menu(&menu)
        .tooltip("HelixNotes")
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.unminimize();
                    let _ = window.set_focus();
                }
            }
            "quit" => {
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let tauri::tray::TrayIconEvent::Click { .. } = event {
                if let Some(window) = tray.app_handle().get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.unminimize();
                    let _ = window.set_focus();
                }
            }
        })
        .build(app)?;

    Ok(())
}

fn percent_decode(input: &str) -> String {
    let mut output = Vec::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(decoded) =
                u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16)
            {
                output.push(decoded);
                i += 3;
                continue;
            }
        }
        output.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&output).to_string()
}

#[cfg(target_os = "macos")]
fn queue_macos_traffic_light_fix(window: &tauri::Window) {
    let window_on_main = window.clone();
    let _ = window.run_on_main_thread(move || {
        fix_macos_traffic_lights(&window_on_main);
    });
}

#[cfg(target_os = "macos")]
fn fix_macos_traffic_lights(window: &tauri::Window) {
    use objc2_app_kit::{NSView, NSWindow, NSWindowButton};

    const TITLEBAR_HEIGHT: f64 = 34.0;
    const LEFT_INSET: f64 = 12.0;

    let ns_window_ptr = match window.ns_window() {
        Ok(ptr) => ptr,
        Err(_) => return,
    };
    let ns_window: &NSWindow = unsafe { &*(ns_window_ptr as *const NSWindow) };

    let close = match ns_window.standardWindowButton(NSWindowButton::CloseButton) {
        Some(button) => button,
        None => return,
    };
    let miniaturize = match ns_window.standardWindowButton(NSWindowButton::MiniaturizeButton) {
        Some(button) => button,
        None => return,
    };
    let zoom = match ns_window.standardWindowButton(NSWindowButton::ZoomButton) {
        Some(button) => button,
        None => return,
    };

    let window_height = ns_window.frame().size.height;
    if window_height <= 0.0 {
        return;
    }

    let close_frame = NSView::frame(&close);
    let button_spacing = NSView::frame(&miniaturize).origin.x - close_frame.origin.x;
    for (index, button) in [&close, &miniaturize, &zoom].into_iter().enumerate() {
        let button_superview = match unsafe { button.superview() } {
            Some(view) => view,
            None => return,
        };
        let mut frame = NSView::frame(button);
        let top_inset = ((TITLEBAR_HEIGHT - frame.size.height) / 2.0).max(0.0);
        let mut target = frame.origin;
        target.y = window_height - top_inset - frame.size.height;
        frame.origin.y = button_superview.convertPoint_fromView(target, None).y;
        frame.origin.x = LEFT_INSET + index as f64 * button_spacing;
        button.setFrameOrigin(frame.origin);
    }
}
