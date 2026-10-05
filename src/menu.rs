//! The menu bar on macOS: the app menu as winit makes it, but with About
//! Ferriteweazle opening the app's own About window.

pub struct MenuBar {
    /// Kept: the menu bar stays up as long as this does.
    _menu: muda::Menu,
    about: muda::MenuId,
}

impl MenuBar {
    /// Puts the menu bar up, in place of winit's. On the main thread, once
    /// the app runs.
    pub fn install() -> Option<MenuBar> {
        use muda::{Menu, MenuItem, PredefinedMenuItem, Submenu};
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
        Some(MenuBar {
            _menu: menu,
            about: about.id().clone(),
        })
    }

    /// Whether About Ferriteweazle was chosen since last asked.
    pub fn about_chosen(&self) -> bool {
        let mut chosen = false;
        while let Ok(event) = muda::MenuEvent::receiver().try_recv() {
            chosen |= event.id == self.about;
        }
        chosen
    }
}
