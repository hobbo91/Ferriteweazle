//! The menu bar on macOS: the app menu as winit makes it, but with About
//! Ferriteweazle opening the app's own About window.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub struct MenuBar {
    /// Kept: the menu bar stays up as long as this does.
    _menu: muda::Menu,
    /// About Ferriteweazle was chosen, until the window asks.
    about: Arc<AtomicBool>,
}

impl MenuBar {
    /// Puts the menu bar up, in place of winit's. On the main thread, once
    /// the app runs. A choice wakes `ctx`'s window, which would otherwise
    /// notice it only at its next frame.
    pub fn install(ctx: &eframe::egui::Context) -> Option<MenuBar> {
        use muda::{Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
        let about = MenuItem::new("About Ferriteweazle", true, None);
        let app = Submenu::with_items(
            "Ferriteweazle",
            true,
            &[
                &about,
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::services(None),
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::hide(Some("Hide Ferriteweazle")),
                &PredefinedMenuItem::hide_others(None),
                &PredefinedMenuItem::show_all(None),
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::quit(Some("Quit Ferriteweazle")),
            ],
        )
        .ok()?;
        let menu = Menu::new();
        menu.append(&app).ok()?;
        menu.init_for_nsapp();
        let chosen = Arc::new(AtomicBool::new(false));
        let (flag, id, ctx) = (chosen.clone(), about.id().clone(), ctx.clone());
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if event.id == id {
                flag.store(true, Ordering::Relaxed);
            }
            ctx.request_repaint();
        }));
        Some(MenuBar {
            _menu: menu,
            about: chosen,
        })
    }

    /// Whether About Ferriteweazle was chosen since last asked.
    pub fn about_chosen(&self) -> bool {
        self.about.swap(false, Ordering::Relaxed)
    }
}
