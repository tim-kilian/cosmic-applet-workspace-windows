// SPDX-License-Identifier: GPL-3.0-only

mod config;
mod wayland;

use std::sync::LazyLock;

use config::{AppletConfig, MAX_TITLE_CHARS, MIN_TITLE_CHARS};
use cosmic::{
    cctk::wayland_protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1,
    Element, app,
    desktop::{
        DesktopEntryCache, DesktopLookupContext, DesktopResolveOptions, IconSourceExt, fde,
        resolve_desktop_entry,
    },
    iced::{
        self, Alignment, Length, Subscription, event, mouse, widget::{row, space, stack},
        window,
    },
    surface::action::{app_popup, destroy_popup},
    theme,
    widget::{self, autosize, container, menu},
};

use wayland::{
    WaylandUpdate, WorkspaceWindow, close_window, focus_window, workspace_windows_subscription,
};

const APP_ID: &str = "io.github.tkilian.CosmicAppletAppTitle";
const CLOSE_ICON_SIZE: u16 = 14;
const EMPTY_TITLE: &str = "Desktop";
const SETTINGS_ICON: &str = "preferences-system-symbolic";
const CONTEXT_MENU_WIDTH: f32 = 220.0;
const SETTINGS_POPUP_WIDTH: f32 = 360.0;

static AUTOSIZE_MAIN_ID: LazyLock<widget::Id> = LazyLock::new(|| widget::Id::new("autosize-main"));

pub fn run() -> cosmic::iced::Result {
    cosmic::applet::run::<Applet>(())
}

#[derive(Clone)]
struct DisplayWindow {
    handle: ExtForeignToplevelHandleV1,
    title: String,
    icon: Option<widget::icon::Handle>,
    is_active: bool,
}

struct Applet {
    config: AppletConfig,
    config_dirty: bool,
    context_menu_popup: Option<window::Id>,
    core: cosmic::app::Core,
    cursor_in_applet: Option<iced::Point>,
    desktop_cache: DesktopEntryCache,
    hovered_window: Option<ExtForeignToplevelHandleV1>,
    open_settings_after_menu: bool,
    settings_popup: Option<window::Id>,
    source_windows: Vec<WorkspaceWindow>,
    windows: Vec<DisplayWindow>,
}

#[derive(Debug, Clone)]
enum Message {
    ClearHoveredWindow(ExtForeignToplevelHandleV1),
    ClearHoveredWindowGlobal,
    CloseWindow(ExtForeignToplevelHandleV1),
    FocusWindow(ExtForeignToplevelHandleV1),
    HoverWindow(ExtForeignToplevelHandleV1),
    OpenContextMenu,
    OpenSettingsPopup,
    PopupClosed(window::Id),
    SetMaxTitleChars(usize),
    SetMiddleClickCloses(bool),
    SetShowAppIcons(bool),
    SetShowHoverCloseButton(bool),
    UpdateAppletCursor(iced::Point),
    Wayland(WaylandUpdate),
}

impl Applet {
    fn persist_config_if_dirty(&mut self) {
        if self.config_dirty {
            self.config.save();
            self.config_dirty = false;
        }
    }

    fn max_chars(&self) -> usize {
        self.config.max_title_chars
    }

    fn resolve_icon(&mut self, window: &WorkspaceWindow) -> Option<widget::icon::Handle> {
        let app_id = window.app_id.as_deref().or(window.identifier.as_deref())?;

        let mut lookup = DesktopLookupContext::new(app_id).with_title(window.title.as_str());
        if let Some(identifier) = window.identifier.as_deref() {
            lookup = lookup.with_identifier(identifier);
        }

        let entry = resolve_desktop_entry(
            &mut self.desktop_cache,
            &lookup,
            &DesktopResolveOptions::default(),
        );
        let icon = fde::IconSource::from_unknown(entry.icon().unwrap_or(&entry.appid));
        Some(icon.as_cosmic_icon())
    }

    fn close_button(
        handle: ExtForeignToplevelHandleV1,
        is_active: bool,
    ) -> Element<'static, Message> {
        widget::button::custom(
            widget::icon::from_name("window-close-symbolic")
                .size(CLOSE_ICON_SIZE)
                .icon(),
        )
        .padding(4)
        .class(close_button_class(is_active))
        .on_press(Message::CloseWindow(handle))
        .into()
    }

    fn rebuild_windows(&mut self) {
        let source_windows = self.source_windows.clone();
        self.windows = source_windows
            .iter()
            .map(|window| DisplayWindow {
                handle: window.handle.clone(),
                title: window.title.clone(),
                icon: if self.config.show_app_icons {
                    self.resolve_icon(window)
                } else {
                    None
                },
                is_active: window.is_active,
            })
            .collect();

        if self
            .hovered_window
            .as_ref()
            .is_some_and(|hovered| !self.windows.iter().any(|window| &window.handle == hovered))
        {
            self.hovered_window = None;
        }
    }

    fn settings_panel(&self) -> Element<'_, Message> {
        let content = widget::container(
            widget::settings::view_column(vec![
                widget::text::title4("Workspace Windows").into(),
                widget::text::caption("Changes apply immediately and are saved automatically.")
                    .into(),
                widget::settings::section()
                    .title("Display")
                    .add(
                        widget::settings::item::builder("Show application icons")
                            .description("Display the desktop icon before each window title.")
                            .toggler(self.config.show_app_icons, Message::SetShowAppIcons),
                    )
                    .add(
                        widget::settings::item::builder("Maximum title length")
                            .description("Limit how many characters each window title can use.")
                            .control(widget::spin_button(
                                self.config.max_title_chars.to_string(),
                                self.config.max_title_chars,
                                1,
                                MIN_TITLE_CHARS,
                                MAX_TITLE_CHARS,
                                Message::SetMaxTitleChars,
                            )),
                    )
                    .into(),
                widget::settings::section()
                    .title("Actions")
                    .add(
                        widget::settings::item::builder("Hover close button")
                            .description("Show the round close button when a tile is hovered.")
                            .toggler(
                                self.config.show_hover_close_button,
                                Message::SetShowHoverCloseButton,
                            ),
                    )
                    .add(
                        widget::settings::item::builder("Middle-click closes windows")
                            .description("Close a window directly by middle-clicking its tile.")
                            .toggler(
                                self.config.middle_click_closes,
                                Message::SetMiddleClickCloses,
                            ),
                    )
                    .into(),
            ])
            .width(Length::Fill),
        )
        .padding(16)
        .width(Length::Fixed(SETTINGS_POPUP_WIDTH));

        self.core.applet.popup_container(content).into()
    }

    fn context_menu_panel(&self) -> Element<'_, Message> {
        let content = container(
            menu::menu_button(vec![
                widget::icon::from_name(SETTINGS_ICON)
                    .size(16)
                    .icon()
                    .into(),
                widget::text("Settings").into(),
                space::horizontal().width(Length::Fill).into(),
            ])
            .on_press(Message::OpenSettingsPopup),
        )
        .padding([8, 0])
        .width(Length::Fixed(CONTEXT_MENU_WIDTH));

        self.core.applet.popup_container(content).into()
    }

    fn open_context_menu_task(&self) -> app::Task<Message> {
        surface_task(app_popup::<Applet>(
            |state: &mut Applet| {
                let new_id = window::Id::unique();
                state.context_menu_popup = Some(new_id);

                let mut popup_settings = state.core.applet.get_popup_settings(
                    state
                        .core
                        .main_window_id()
                        .expect("applet main window missing"),
                    new_id,
                    None,
                    None,
                    None,
                );

                if let Some(position) = state.cursor_in_applet {
                    popup_settings.positioner.anchor_rect = iced::Rectangle {
                        x: position.x.round() as i32,
                        y: position.y.round() as i32,
                        width: 1,
                        height: 1,
                    };
                }

                popup_settings
            },
            Some(Box::new(|state: &Applet| {
                state.context_menu_panel().map(cosmic::Action::App)
            })),
        ))
    }

    fn open_settings_task(&self) -> app::Task<Message> {
        surface_task(app_popup::<Applet>(
            |state: &mut Applet| {
                let new_id = window::Id::unique();
                state.settings_popup = Some(new_id);

                let mut popup_settings = state.core.applet.get_popup_settings(
                    state
                        .core
                        .main_window_id()
                        .expect("applet main window missing"),
                    new_id,
                    None,
                    None,
                    None,
                );

                if let Some(position) = state.cursor_in_applet {
                    popup_settings.positioner.anchor_rect = iced::Rectangle {
                        x: position.x.round() as i32,
                        y: position.y.round() as i32,
                        width: 1,
                        height: 1,
                    };
                }

                popup_settings
            },
            Some(Box::new(|state: &Applet| {
                state.settings_panel().map(cosmic::Action::App)
            })),
        ))
    }

    fn window_tile(&self, window: &DisplayWindow, icon_size: f32) -> Element<'_, Message> {
        let text = truncate_title(&window.title, self.max_chars());
        let mut content = row![].align_y(Alignment::Center).spacing(4);

        if let Some(icon) = window.icon.clone() {
            content = content.push(
                widget::icon(icon)
                    .width(Length::Fixed(icon_size))
                    .height(Length::Fixed(icon_size)),
            );
        }

        content = content.push(self.core.applet.text(text));

        let is_active = window.is_active;
        let is_hovered = self
            .hovered_window
            .as_ref()
            .is_some_and(|hovered| hovered == &window.handle);
        let handle = window.handle.clone();
        let hover_handle = handle.clone();
        let hover_move_handle = handle.clone();
        let hover_clear_handle = handle.clone();
        let close_handle = handle.clone();
        let preview = container(content)
            .padding([2, 8])
            .class(theme::Container::custom(move |theme| {
                let cosmic = theme.cosmic();
                let (background, foreground, border_color, border_width) = if is_active {
                    (
                        if is_hovered {
                            cosmic.accent_button.hover.into()
                        } else {
                            cosmic.accent_button.base.into()
                        },
                        cosmic.accent_button.on.into(),
                        if is_hovered {
                            cosmic.accent.base.into()
                        } else {
                            iced::Color::TRANSPARENT
                        },
                        if is_hovered { 1.0 } else { 0.0 },
                    )
                } else {
                    (
                        if is_hovered {
                            cosmic.background.component.hover.into()
                        } else {
                            cosmic.background.component.base.into()
                        },
                        cosmic.background.component.on.into(),
                        if is_hovered {
                            cosmic.bg_divider().into()
                        } else {
                            iced::Color::TRANSPARENT
                        },
                        if is_hovered { 1.0 } else { 0.0 },
                    )
                };

                container::Style {
                    icon_color: Some(foreground),
                    text_color: Some(foreground),
                    background: Some(iced::Background::Color(background)),
                    border: iced::Border {
                        radius: cosmic.corner_radii.radius_s.into(),
                        color: border_color,
                        width: border_width,
                        ..Default::default()
                    },
                    shadow: Default::default(),
                    snap: true,
                }
            }));

        let close_button_overlay: Element<'_, Message> =
            if self.config.show_hover_close_button && is_hovered {
                widget::mouse_area(
                    row![
                        space::horizontal().width(Length::Fill),
                        container(Self::close_button(close_handle, is_active)).padding([0, 4])
                    ]
                    .align_y(Alignment::Center)
                    .width(Length::Fill)
                    .height(Length::Fill),
                )
                .interaction(mouse::Interaction::Idle)
                .on_exit(Message::ClearHoveredWindow(hover_clear_handle))
                .into()
            } else {
                row![].width(Length::Fill).height(Length::Fill).into()
            };

        let tile = widget::mouse_area(stack![preview, close_button_overlay])
            .interaction(mouse::Interaction::Idle)
            .on_enter(Message::HoverWindow(hover_handle))
            .on_move(move |_| Message::HoverWindow(hover_move_handle.clone()))
            .on_exit(Message::ClearHoveredWindow(handle.clone()))
            .on_press(Message::FocusWindow(handle.clone()));

        let tile = if self.config.middle_click_closes {
            tile.on_middle_press(Message::CloseWindow(handle.clone()))
        } else {
            tile
        };

        tile.into()
    }

    fn empty_tile(&self) -> Element<'_, Message> {
        container(self.core.applet.text(EMPTY_TITLE))
            .padding([2, 8])
            .class(theme::Container::custom(move |theme| {
                let cosmic = theme.cosmic();
                let background = cosmic.background.component.base.into();
                let foreground = cosmic.background.component.on.into();

                container::Style {
                    icon_color: Some(foreground),
                    text_color: Some(foreground),
                    background: Some(iced::Background::Color(background)),
                    border: iced::Border {
                        radius: cosmic.corner_radii.radius_s.into(),
                        ..Default::default()
                    },
                    shadow: Default::default(),
                    snap: true,
                }
            }))
            .into()
    }
}

impl cosmic::Application for Applet {
    type Message = Message;
    type Executor = cosmic::SingleThreadExecutor;
    type Flags = ();

    const APP_ID: &'static str = APP_ID;

    fn init(core: cosmic::app::Core, _flags: Self::Flags) -> (Self, app::Task<Self::Message>) {
        let config = AppletConfig::load();
        (
            Self {
                config,
                config_dirty: false,
                context_menu_popup: None,
                core,
                cursor_in_applet: None,
                desktop_cache: DesktopEntryCache::new(fde::get_languages_from_env()),
                hovered_window: None,
                open_settings_after_menu: false,
                settings_popup: None,
                source_windows: Vec::new(),
                windows: Vec::new(),
            },
            app::Task::none(),
        )
    }

    fn core(&self) -> &cosmic::app::Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut cosmic::app::Core {
        &mut self.core
    }

    fn style(&self) -> Option<iced::theme::Style> {
        Some(cosmic::applet::style())
    }

    fn update(&mut self, message: Self::Message) -> app::Task<Self::Message> {
        match message {
            Message::ClearHoveredWindow(handle) => {
                if self
                    .hovered_window
                    .as_ref()
                    .is_some_and(|hovered| hovered == &handle)
                {
                    self.hovered_window = None;
                }
            }
            Message::ClearHoveredWindowGlobal => {
                self.hovered_window = None;
                self.cursor_in_applet = None;
            }
            Message::CloseWindow(handle) => {
                close_window(handle);
            }
            Message::FocusWindow(handle) => {
                focus_window(handle);
            }
            Message::HoverWindow(handle) => {
                self.hovered_window = Some(handle);
            }
            Message::OpenContextMenu => {
                if self.settings_popup.is_some() || self.open_settings_after_menu {
                    return app::Task::none();
                }

                if let Some(id) = self.context_menu_popup {
                    return surface_task(destroy_popup(id));
                }

                return self.open_context_menu_task();
            }
            Message::OpenSettingsPopup => {
                if self.settings_popup.is_some() || self.open_settings_after_menu {
                    return app::Task::none();
                }

                if let Some(menu_id) = self.context_menu_popup {
                    self.open_settings_after_menu = true;
                    return surface_task(destroy_popup(menu_id));
                }

                return self.open_settings_task();
            }
            Message::PopupClosed(id) => {
                if self.context_menu_popup == Some(id) {
                    self.context_menu_popup = None;
                    if self.open_settings_after_menu {
                        self.open_settings_after_menu = false;
                        return self.open_settings_task();
                    }
                }
                if self.settings_popup == Some(id) {
                    self.settings_popup = None;
                    self.persist_config_if_dirty();
                }
            }
            Message::SetMaxTitleChars(value) => {
                let value = value.clamp(MIN_TITLE_CHARS, MAX_TITLE_CHARS);
                if self.config.max_title_chars != value {
                    self.config.max_title_chars = value;
                    self.config_dirty = true;
                }
            }
            Message::SetMiddleClickCloses(value) => {
                if self.config.middle_click_closes != value {
                    self.config.middle_click_closes = value;
                    self.config_dirty = true;
                }
            }
            Message::SetShowAppIcons(value) => {
                if self.config.show_app_icons != value {
                    self.config.show_app_icons = value;
                    self.config_dirty = true;
                    self.rebuild_windows();
                }
            }
            Message::SetShowHoverCloseButton(value) => {
                if self.config.show_hover_close_button != value {
                    self.config.show_hover_close_button = value;
                    self.config_dirty = true;
                }
            }
            Message::UpdateAppletCursor(position) => {
                self.cursor_in_applet = Some(position);
            }
            Message::Wayland(update) => match update {
                WaylandUpdate::WorkspaceWindows(windows) => {
                    self.source_windows = windows;
                    self.rebuild_windows();
                }
                WaylandUpdate::Finished => {
                    tracing::error!("Wayland subscription ended");
                }
            },
        }

        app::Task::none()
    }

    fn subscription(&self) -> Subscription<Self::Message> {
        Subscription::batch([
            workspace_windows_subscription().map(Message::Wayland),
            event::listen_with(|event, _, _| match event {
                iced::Event::Mouse(mouse::Event::CursorLeft) => {
                    Some(Message::ClearHoveredWindowGlobal)
                }
                _ => None,
            }),
        ])
    }

    fn view(&self) -> Element<'_, Self::Message> {
        let height = (self.core.applet.suggested_size(true).1
            + 2 * self.core.applet.suggested_padding(true).1) as f32;
        let icon_size = self.core.applet.suggested_size(true).0 as f32;
        let mut content = row![].align_y(Alignment::Center).spacing(6);

        if self.windows.is_empty() {
            content = content.push(self.empty_tile());
        } else {
            for window in &self.windows {
                content = content.push(self.window_tile(window, icon_size));
            }
        }

        content = content.push(space::vertical().height(Length::Fixed(height)));

        let content = container(content).padding([0, self.core.applet.suggested_padding(true).0]);
        widget::mouse_area(autosize::autosize(content, AUTOSIZE_MAIN_ID.clone()))
            .interaction(mouse::Interaction::Idle)
            .on_move(Message::UpdateAppletCursor)
            .on_right_press(Message::OpenContextMenu)
            .into()
    }

    fn view_window(&self, id: window::Id) -> Element<'_, Self::Message> {
        if self.settings_popup == Some(id) {
            self.settings_panel()
        } else if self.context_menu_popup == Some(id) {
            self.context_menu_panel()
        } else {
            widget::text::body("").into()
        }
    }

    fn on_close_requested(&self, id: window::Id) -> Option<Self::Message> {
        Some(Message::PopupClosed(id))
    }
}

fn surface_task(action: cosmic::surface::Action) -> app::Task<Message> {
    cosmic::task::message(cosmic::Action::Cosmic(cosmic::app::Action::Surface(action)))
}

fn truncate_title(title: &str, max_chars: usize) -> String {
    let char_count = title.chars().count();
    if char_count <= max_chars {
        return title.to_owned();
    }

    let keep = max_chars.saturating_sub(3);
    let mut truncated = title.chars().take(keep).collect::<String>();
    truncated.push_str("...");
    truncated
}

fn close_button_class(is_active: bool) -> theme::Button {
    theme::Button::Custom {
        active: Box::new(move |_, theme| close_button_style(theme, is_active, 0.0)),
        disabled: Box::new(move |theme| close_button_style(theme, is_active, 0.0)),
        hovered: Box::new(move |_, theme| close_button_style(theme, is_active, 0.14)),
        pressed: Box::new(move |_, theme| close_button_style(theme, is_active, 0.22)),
    }
}

fn close_button_style(
    theme: &cosmic::Theme,
    is_active: bool,
    background_alpha: f32,
) -> widget::button::Style {
    let cosmic = theme.cosmic();
    let foreground = if is_active {
        cosmic.accent_button.on.into()
    } else {
        cosmic.background.component.on.into()
    };
    let background = (background_alpha > 0.0)
        .then(|| iced::Background::Color(with_alpha(foreground, background_alpha)));

    widget::button::Style {
        shadow_offset: iced::Vector::default(),
        background,
        overlay: None,
        border_radius: cosmic.corner_radii.radius_xl.into(),
        border_width: 0.0,
        border_color: iced::Color::TRANSPARENT,
        outline_width: 0.0,
        outline_color: iced::Color::TRANSPARENT,
        icon_color: Some(foreground),
        text_color: Some(foreground),
    }
}

fn with_alpha(mut color: iced::Color, alpha: f32) -> iced::Color {
    color.a = alpha;
    color
}
