//! Viewer "Íris" — GTK4 + libadwaita.
//!
//! MVP+zoom/pan: `gtk::Picture` wrapped in `gtk::ScrolledWindow`. `Fit` mode
//! uses `ContentFit::Contain` and lets the picture ride the viewport; the
//! moment the user zooms, we switch to `Fill` with an explicit size request
//! so the scroll view can give actual scrollbars and pan behaviour.
//!
//! Gesture-zoom (pinch), cursor-centred zoom, film strip, EXIF sidebar and
//! the glycin decoder swap are planned for the M2 milestone.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio, glib, graphene};
use gtk4 as gtk;
use libadwaita as adw;

const APP_ID: &str = "com.biglinux.Iris";
const ZOOM_STEP: f64 = 1.25;
const MIN_ZOOM: f64 = 0.05;
const MAX_ZOOM: f64 = 32.0;
/// Top-of-window reveal zone for the header bar, in pixels.
const TOP_EDGE_PX: f64 = 60.0;
/// Bottom-of-window reveal zone for the film strip, in pixels. Slightly
/// taller than the strip itself (96 px) so the user can "catch" it.
const BOTTOM_EDGE_PX: f64 = 110.0;
/// Hide delay for header / strip after the cursor leaves the edge zone.
const EDGE_HIDE_MS: u64 = 900;
/// OSD pill visible duration after a nav/zoom pulse, mpv-style.
const OSD_PULSE_MS: u64 = 1200;
/// Idle time before the cursor itself blinks out (mouse still, no key).
const CURSOR_HIDE_MS: u64 = 2000;
/// Slideshow step between auto-advances (Space toggles).
const SLIDESHOW_INTERVAL_S: u32 = 5;

/// CSS applied app-wide. Theater is on by default (see `build_window`);
/// `.theater-off` is the escape hatch that restores Adwaita chrome when
/// the user toggles theater off.
const THEATER_CSS: &str = "
.theater, .theater scrolledwindow, .theater scrolledwindow > viewport {
    background-color: #000;
}
.theater headerbar {
    background-color: rgba(0, 0, 0, 0.55);
    box-shadow: none;
}
.film-strip { padding: 4px 8px; }
.film-strip button { padding: 2px; min-height: 0; min-width: 0; }
.film-strip button.active-thumb {
    outline: 2px solid @accent_color;
    outline-offset: -2px;
}
";

/// Pixel size of each thumbnail in the film strip (longest edge).
const THUMB_EDGE: u32 = 80;

/// Start the viewer event loop. Returns the exit code GTK produced.
pub fn run_viewer(files: Vec<PathBuf>) -> i32 {
    // NON_UNIQUE: sem isso o GApplication tenta single-instance via D-Bus
    // e, se uma janela do BigIris tá se fechando no momento em que outra
    // tenta abrir (ex.: dashboard spawna novo processo e fecha), a nova
    // ativação é descartada. NON_UNIQUE garante que cada `bigiris …`
    // seja um processo independente com seu próprio event loop.
    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();

    let files = Rc::new(files);
    app.connect_activate(move |app| {
        install_theater_css();
        build_window(app, files.clone()).present();
    });

    // Empty argv: clap already consumed the interesting bits and we don't want
    // GTK to re-parse our positional file paths as GIO URLs.
    let code = app.run_with_args::<&str>(&[]);
    i32::from(u8::from(code))
}

/// Mutable viewer state shared between the window and its controllers.
///
/// The UI is edge-triggered: cursor near the top reveals the header,
/// near the bottom reveals the film strip, and nav/zoom pulses the OSD.
/// Everywhere else is pure image, so timers are kept per-zone.
struct ViewerState {
    files: Rc<Vec<PathBuf>>,
    idx: usize,
    picture: gtk::Picture,
    scroller: gtk::ScrolledWindow,
    title: adw::WindowTitle,
    window: adw::ApplicationWindow,
    zoom: ZoomState,
    /// Natural pixel dimensions of the currently-loaded image.
    natural: (u32, u32),
    /// Toolbar view holding the header bar; revealed/hidden by edge-reveal.
    toolbar_view: adw::ToolbarView,
    /// Bottom-center OSD revealer (mpv-style pill: name · idx · zoom).
    osd_revealer: gtk::Revealer,
    /// Label inside the OSD revealer.
    osd_label: gtk::Label,
    /// Film strip revealer (SlideUp). Separate from OSD so each zone has
    /// its own lifecycle.
    strip_revealer: gtk::Revealer,
    /// Film strip scrolled container (hidden when files.len() < 2).
    film_strip_scroll: gtk::ScrolledWindow,
    /// One button per file so we can flip `.active-thumb` on navigation.
    film_strip_buttons: Vec<gtk::Button>,
    /// When true (accessibility override), chrome stays pinned visible.
    ui_pinned: bool,
    /// Theater mode darkens the chrome around the picture, no fullscreen.
    /// Enabled by default — the photo-viewer paradigm.
    theater_enabled: bool,
    /// Pending hide timers, one per zone, so each fades on its own clock.
    top_hide_source: Option<glib::SourceId>,
    bottom_hide_source: Option<glib::SourceId>,
    osd_hide_source: Option<glib::SourceId>,
    cursor_hide_source: Option<glib::SourceId>,
    /// Slideshow auto-advance tick (Space toggles). `None` = stopped.
    slideshow_source: Option<glib::SourceId>,
    /// DrawingArea com o overlay de histograma (canto inferior-direito).
    /// Visibilidade controlada pela tecla `G` — oculto por padrão.
    histogram_area: gtk::DrawingArea,
    /// Histograma RGB da imagem atual: `[0..256]=R, [256..512]=G, [512..768]=B`.
    /// Recalculado em cada `update_display`.
    histogram: [u32; 768],
    /// Dialog de propriedades atualmente aberto, se houver. Evita que
    /// cliques repetidos no botão `i` empilhem janelas.
    props_dialog: Option<adw::ApplicationWindow>,
    /// Edições pendentes (rotate/flip) aplicadas in-memory sobre a
    /// imagem decodificada. Vazio = arquivo "limpo". `Ctrl+S` grava e
    /// limpa; navegar de arquivo também limpa.
    pending_ops: Vec<EditOp>,
}

/// Edição in-memory no estilo Loupe: o user gira/espelha sem tocar o
/// arquivo; `•` aparece no título e só no `Ctrl+S` o disco é gravado.
/// As ops são acumuladas em `ViewerState.pending_ops` e reaplicadas por
/// cima do decode original a cada push.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditOp {
    RotateCw,
    RotateCcw,
    Rotate180,
    FlipH,
    FlipV,
}

#[derive(Debug, Clone, Copy)]
enum ZoomState {
    /// Fit inside viewport, preserve aspect, never enlarge past 1:1.
    Fit,
    /// Explicit scale factor; `1.0` = actual pixel size.
    Scale(f64),
}

fn build_window(app: &adw::Application, files: Rc<Vec<PathBuf>>) -> adw::ApplicationWindow {
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .default_width(1024)
        .default_height(720)
        .build();

    let title = adw::WindowTitle::new("BigIris", "");
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&title));

    // As ações do menu hambúrguer precisam do `state`, então criamos o
    // botão depois que o state existe — veja mais abaixo. Header fica com
    // os quick-buttons (info/fullscreen) agora.

    let picture = gtk::Picture::new();
    picture.set_content_fit(gtk::ContentFit::Contain);
    picture.set_hexpand(true);
    picture.set_vexpand(true);
    picture.set_halign(gtk::Align::Center);
    picture.set_valign(gtk::Align::Center);

    let scroller = gtk::ScrolledWindow::builder()
        .hexpand(true)
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .child(&picture)
        .build();

    let status = build_dashboard(&window);

    // Stack between empty-state and the viewport so we don't flash a blank
    // area when the app is launched without files.
    let stack = gtk::Stack::new();
    stack.add_named(&status, Some("empty"));
    stack.add_named(&scroller, Some("picture"));

    // OSD pill (mpv-style): filename · index · zoom. Hidden by default,
    // pulses on nav/zoom. Crossfade in, slide-down on hide so it feels
    // ambient rather than chrome-y.
    let osd_label = gtk::Label::builder()
        .label("")
        .css_classes(["osd", "title-4"])
        .margin_start(12)
        .margin_end(12)
        .margin_top(6)
        .margin_bottom(6)
        .build();
    let osd_bin = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    osd_bin.add_css_class("osd");
    osd_bin.add_css_class("card");
    osd_bin.set_halign(gtk::Align::Center);
    osd_bin.append(&osd_label);
    let osd_revealer = gtk::Revealer::builder()
        .transition_type(gtk::RevealerTransitionType::Crossfade)
        .transition_duration(220)
        .reveal_child(false)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::End)
        .margin_bottom(120)
        .can_target(false)
        .child(&osd_bin)
        .build();

    // Film strip: horizontal row of thumbnail buttons, revealed only when
    // the cursor touches the bottom edge zone. Thumbs are built up lazily
    // (one per idle tick) so huge selections don't stall startup.
    let film_strip_box = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    film_strip_box.add_css_class("film-strip");
    let film_strip_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Never)
        .child(&film_strip_box)
        .height_request(96)
        .build();

    let mut film_strip_buttons: Vec<gtk::Button> = Vec::with_capacity(files.len());
    for _ in files.iter() {
        let picture = gtk::Picture::new();
        picture.set_content_fit(gtk::ContentFit::Contain);
        picture.set_size_request(THUMB_EDGE as i32, THUMB_EDGE as i32);
        let btn = gtk::Button::builder().child(&picture).build();
        btn.add_css_class("flat");
        film_strip_box.append(&btn);
        film_strip_buttons.push(btn);
    }

    let strip_revealer = gtk::Revealer::builder()
        .transition_type(gtk::RevealerTransitionType::SlideUp)
        .transition_duration(220)
        .reveal_child(false)
        .valign(gtk::Align::End)
        .halign(gtk::Align::Fill)
        .child(&film_strip_scroll)
        .build();
    // If there are fewer than two files the strip is meaningless — keep
    // it collapsed regardless of edge reveals.
    if files.len() < 2 {
        strip_revealer.set_can_target(false);
        strip_revealer.set_visible(false);
    }

    // Histograma RGB (overlay canto superior-direito). Oculto por padrão;
    // tecla `G` alterna. Tamanho fixo porque o frame sempre renderiza
    // 256 bins — deixar o widget esticar distorceria as colunas.
    let histogram_area = gtk::DrawingArea::builder()
        .width_request(260)
        .height_request(104)
        .halign(gtk::Align::End)
        .valign(gtk::Align::Start)
        .margin_top(8)
        .margin_end(8)
        .visible(false)
        .build();
    histogram_area.add_css_class("histogram-overlay");

    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&stack));
    overlay.add_overlay(&osd_revealer);
    overlay.add_overlay(&strip_revealer);
    overlay.add_overlay(&histogram_area);

    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&header);
    toolbar_view.set_content(Some(&overlay));
    // Start with the header hidden — edge-triggered reveal handles the rest.
    toolbar_view.set_reveal_top_bars(false);
    window.set_content(Some(&toolbar_view));

    // Theater mode on by default: photos look best on black, and it's what
    // Apple Photos / Lightroom / mpv converge on for viewing surfaces.
    window.add_css_class("theater");

    let state = Rc::new(RefCell::new(ViewerState {
        files: files.clone(),
        idx: 0,
        picture: picture.clone(),
        scroller: scroller.clone(),
        title: title.clone(),
        window: window.clone(),
        zoom: ZoomState::Fit,
        natural: (0, 0),
        toolbar_view: toolbar_view.clone(),
        osd_revealer: osd_revealer.clone(),
        osd_label: osd_label.clone(),
        strip_revealer: strip_revealer.clone(),
        film_strip_scroll: film_strip_scroll.clone(),
        film_strip_buttons: film_strip_buttons.clone(),
        ui_pinned: false,
        theater_enabled: true,
        top_hide_source: None,
        bottom_hide_source: None,
        osd_hide_source: None,
        cursor_hide_source: None,
        slideshow_source: None,
        histogram_area: histogram_area.clone(),
        histogram: [0u32; 768],
        props_dialog: None,
        pending_ops: Vec::new(),
    }));

    // draw callback lê o snapshot mais recente do histograma em state.
    {
        let state = state.clone();
        histogram_area.set_draw_func(move |_, cr, w, h| {
            draw_histogram(&state, cr, w, h);
        });
    }

    // Quick-buttons do header (esquerda) e menu hamburger (direita).
    // Registrados aqui porque todos fecham sobre `state`.
    install_viewer_actions(app, &window, &state);
    let info_btn = gtk::Button::from_icon_name("dialog-information-symbolic");
    info_btn.set_tooltip_text(Some("Propriedades da imagem (Ctrl+I)"));
    info_btn.set_action_name(Some("win.info"));
    // Só faz sentido com imagem carregada — na dashboard fica escondido.
    // Fullscreen fica como F11 e double-click, sem botão dedicado no header
    // (Loupe parity: libadwaita já dá minimize/maximize/close no lado direito).
    let has_files = !state.borrow().files.is_empty();
    info_btn.set_visible(has_files);
    header.pack_start(&info_btn);
    header.pack_end(&build_header_menu_button(&state));

    if has_files {
        stack.set_visible_child_name("picture");
        update_display(&state);
    } else {
        stack.set_visible_child_name("empty");
        title.set_title("BigIris");
    }

    // Keyboard: file nav, zoom presets, fullscreen, quit.
    let key_controller = gtk::EventControllerKey::new();
    {
        let state = state.clone();
        key_controller
            .connect_key_pressed(move |_, key, _code, modifier| handle_key(&state, key, modifier));
    }
    window.add_controller(key_controller);

    // Mouse-wheel zoom — live on the window so revealed chrome (film strip,
    // header) doesn't steal wheel events. Capture phase pre-empts the
    // ScrolledWindow's own scroll-to-scroll handler on the image area.
    let scroll_controller =
        gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
    scroll_controller.set_propagation_phase(gtk::PropagationPhase::Capture);
    {
        let state = state.clone();
        let scroller = scroller.clone();
        scroll_controller.connect_scroll(move |controller, _dx, dy| {
            // The scroll controller lives on the window, so event.position()
            // is in window-root coords. Convert to scroller-local coords so
            // adjust_zoom can anchor the re-scroll on the pixel under the
            // pointer: root.compute_point(&scroller, pt) — NOT the reverse.
            let cursor =
                controller.current_event().and_then(|e| e.position()).and_then(|(wx, wy)| {
                    let root = scroller.root()?;
                    let pt = graphene::Point::new(wx as f32, wy as f32);
                    root.compute_point(&scroller, &pt).map(|p| (f64::from(p.x()), f64::from(p.y())))
                });
            if dy < -0.01 {
                adjust_zoom(&state, ZOOM_STEP, cursor);
                pulse_osd(&state);
                glib::Propagation::Stop
            } else if dy > 0.01 {
                adjust_zoom(&state, 1.0 / ZOOM_STEP, cursor);
                pulse_osd(&state);
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
    }
    window.add_controller(scroll_controller);

    // Double-click anywhere on the image → toggle fullscreen (matches the
    // F11 shortcut). Uses GestureClick with n-press=2 so single clicks still
    // fall through to the drag-to-pan gesture.
    let double_click = gtk::GestureClick::builder().button(1).build();
    double_click.set_propagation_phase(gtk::PropagationPhase::Bubble);
    {
        let state = state.clone();
        double_click.connect_pressed(move |_, n_press, _, _| {
            if n_press == 2 {
                let win = state.borrow().window.clone();
                win.set_fullscreened(!win.is_fullscreen());
            }
        });
    }
    scroller.add_controller(double_click);

    // Drag-to-pan with any mouse button. GestureDrag gives us deltas relative
    // to the drag start, so we just save the scroller's adjustment values at
    // `drag_begin` and subtract from there on every `drag_update`.
    let drag = gtk::GestureDrag::builder().button(0).build();
    drag.set_propagation_phase(gtk::PropagationPhase::Capture);
    let drag_anchor: Rc<Cell<(f64, f64)>> = Rc::new(Cell::new((0.0, 0.0)));
    {
        let scr = scroller.clone();
        let anchor = drag_anchor.clone();
        drag.connect_drag_begin(move |_, _, _| {
            anchor.set((scr.hadjustment().value(), scr.vadjustment().value()));
        });
    }
    {
        let scr = scroller.clone();
        let anchor = drag_anchor.clone();
        drag.connect_drag_update(move |_, dx, dy| {
            let (hx0, vy0) = anchor.get();
            scr.hadjustment().set_value(hx0 - dx);
            scr.vadjustment().set_value(vy0 - dy);
        });
    }
    scroller.add_controller(drag);

    // Film strip: wire each thumb button to jump to its index.
    for (i, btn) in film_strip_buttons.iter().enumerate() {
        let state = state.clone();
        btn.connect_clicked(move |_| {
            {
                let mut s = state.borrow_mut();
                if s.idx == i {
                    return;
                }
                s.idx = i;
                s.zoom = ZoomState::Fit;
            }
            update_display(&state);
            pulse_osd(&state);
        });
    }

    // Lazy-populate thumbnails via the GLib main loop: one file per idle
    // tick keeps the UI responsive even for huge selections. Bail quietly
    // if a file fails to decode; the slot stays empty rather than crashing.
    {
        let files = files.clone();
        let buttons = film_strip_buttons.clone();
        let mut queue: std::collections::VecDeque<usize> = (0..files.len()).collect();
        glib::idle_add_local(move || {
            let Some(idx) = queue.pop_front() else {
                return glib::ControlFlow::Break;
            };
            if let Some(btn) = buttons.get(idx) {
                if let Some(pic) = btn.child().and_downcast::<gtk::Picture>() {
                    if let Ok(tex) = load_thumbnail(&files[idx]) {
                        pic.set_paintable(Some(&tex));
                    }
                }
            }
            glib::ControlFlow::Continue
        });
    }

    // Edge-triggered reveal: cursor near the top shows the header, near
    // the bottom shows the film strip, everywhere else only restores the
    // cursor. Each zone has its own hide timer.
    let motion = gtk::EventControllerMotion::new();
    {
        let state = state.clone();
        motion.connect_motion(move |_, _x, y| on_pointer_motion(&state, y));
    }
    window.add_controller(motion);

    // Kick off: chrome hidden, OSD pulse so the user sees which file is
    // loaded, cursor visible until the first idle tick.
    refresh_film_strip_active(&state);
    refresh_osd(&state);
    pulse_osd(&state);
    schedule_cursor_hide(&state);

    window
}

/// Decode `path` and build a small GDK texture for the film strip.
/// Uses [`image::GenericImageView::thumbnail`] — fast, avoids loading the
/// full-res pixels into GPU memory.
fn load_thumbnail(path: &Path) -> Result<gdk::Texture, Box<dyn std::error::Error>> {
    let img = image::open(path)?;
    let thumb = img.thumbnail(THUMB_EDGE, THUMB_EDGE).to_rgba8();
    let (w, h) = thumb.dimensions();
    let stride = (w as usize) * 4;
    let bytes = glib::Bytes::from_owned(thumb.into_raw());
    let tex =
        gdk::MemoryTexture::new(w as i32, h as i32, gdk::MemoryFormat::R8g8b8a8, &bytes, stride);
    Ok(tex.upcast())
}

/// Toggle `.active-thumb` so the film strip mirrors the current selection.
/// Also autoscrolls the strip so the active thumb stays in view; uses the
/// button's computed bounds (allocation() was deprecated in GTK 4.12).
fn refresh_film_strip_active(state: &Rc<RefCell<ViewerState>>) {
    let s = state.borrow();
    let strip = &s.film_strip_scroll;
    let viewport_w = strip.width() as f64;
    for (i, btn) in s.film_strip_buttons.iter().enumerate() {
        if i == s.idx {
            btn.add_css_class("active-thumb");
            // compute_bounds returns coords in the parent (the HBox inside
            // the scroll's viewport); that's what hadjustment speaks.
            if let Some(parent) = btn.parent() {
                if let Some(rect) = btn.compute_bounds(&parent) {
                    if rect.width() > 0.0 {
                        let target = rect.x() as f64 - (viewport_w - rect.width() as f64) / 2.0;
                        strip.hadjustment().set_value(target.max(0.0));
                    }
                }
            }
        } else {
            btn.remove_css_class("active-thumb");
        }
    }
}

/// Register a CssProvider carrying [`THEATER_CSS`] on the default display.
/// Runs once per activation; duplicate calls are harmless because the same
/// provider just replaces the previous rules.
fn install_theater_css() {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(THEATER_CSS);
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

/// Manual film-strip reveal (F key). Just forwards to [`reveal_bottom`]
/// so it follows the same edge-hide timing as the hover path.
fn toggle_film_strip(state: &Rc<RefCell<ViewerState>>) {
    if state.borrow().film_strip_buttons.len() < 2 {
        return;
    }
    let currently_revealed = state.borrow().strip_revealer.reveals_child();
    if currently_revealed {
        hide_bottom(state);
    } else {
        reveal_bottom(state);
    }
}

/// Flip theater mode. Adds/removes the `.theater` CSS class on the window
/// so the scoped rules in [`THEATER_CSS`] kick in.
fn toggle_theater(state: &Rc<RefCell<ViewerState>>) {
    let (window, enable) = {
        let mut s = state.borrow_mut();
        s.theater_enabled = !s.theater_enabled;
        (s.window.clone(), s.theater_enabled)
    };
    if enable {
        window.add_css_class("theater");
    } else {
        window.remove_css_class("theater");
    }
}

/// Compose the OSD pill text for the current file/zoom, mpv-style:
/// `name.jpg · 3/12 · 85%`.
fn osd_text(state: &Rc<RefCell<ViewerState>>) -> String {
    let s = state.borrow();
    if s.files.is_empty() {
        return String::new();
    }
    let path = &s.files[s.idx];
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    let zoom = match s.zoom {
        ZoomState::Fit => "Ajustar".to_string(),
        ZoomState::Scale(x) => format!("{}%", (x * 100.0).round() as i32),
    };
    format!("{} · {}/{} · {}", name, s.idx + 1, s.files.len(), zoom)
}

/// Sync the OSD label to the current file/zoom without touching visibility.
fn refresh_osd(state: &Rc<RefCell<ViewerState>>) {
    let text = osd_text(state);
    let s = state.borrow();
    s.osd_label.set_label(&text);
    s.osd_revealer.set_visible(!text.is_empty());
}

/// Pointer moved — route to the appropriate zone. Called on every motion
/// event, so keep the work cheap: it's mostly math + timer juggling.
fn on_pointer_motion(state: &Rc<RefCell<ViewerState>>, y: f64) {
    // Always restore the cursor and restart its own idle timer.
    {
        let s = state.borrow();
        s.window.set_cursor(None);
    }
    cancel_cursor_hide(state);
    schedule_cursor_hide(state);

    let (window_h, pinned) = {
        let s = state.borrow();
        (s.window.height() as f64, s.ui_pinned)
    };
    if pinned {
        reveal_top(state);
        reveal_bottom(state);
        return;
    }
    if y <= TOP_EDGE_PX {
        reveal_top(state);
    } else if y >= window_h - BOTTOM_EDGE_PX {
        reveal_bottom(state);
    }
}

/// Slide the header bar into view and (re)arm its hide timer.
fn reveal_top(state: &Rc<RefCell<ViewerState>>) {
    {
        let s = state.borrow();
        s.toolbar_view.set_reveal_top_bars(true);
    }
    cancel_top_hide(state);
    if state.borrow().ui_pinned {
        return;
    }
    let clone = state.clone();
    let id =
        glib::timeout_add_local_once(std::time::Duration::from_millis(EDGE_HIDE_MS), move || {
            hide_top(&clone)
        });
    state.borrow_mut().top_hide_source = Some(id);
}

fn hide_top(state: &Rc<RefCell<ViewerState>>) {
    let mut s = state.borrow_mut();
    s.top_hide_source = None;
    s.toolbar_view.set_reveal_top_bars(false);
}

fn cancel_top_hide(state: &Rc<RefCell<ViewerState>>) {
    if let Some(id) = state.borrow_mut().top_hide_source.take() {
        id.remove();
    }
}

/// Slide the film strip into view (skips the motion when files < 2) and
/// (re)arm its hide timer.
fn reveal_bottom(state: &Rc<RefCell<ViewerState>>) {
    {
        let s = state.borrow();
        if s.film_strip_buttons.len() < 2 {
            return;
        }
        s.strip_revealer.set_reveal_child(true);
    }
    cancel_bottom_hide(state);
    if state.borrow().ui_pinned {
        return;
    }
    let clone = state.clone();
    let id =
        glib::timeout_add_local_once(std::time::Duration::from_millis(EDGE_HIDE_MS), move || {
            hide_bottom(&clone)
        });
    state.borrow_mut().bottom_hide_source = Some(id);
}

fn hide_bottom(state: &Rc<RefCell<ViewerState>>) {
    let mut s = state.borrow_mut();
    s.bottom_hide_source = None;
    s.strip_revealer.set_reveal_child(false);
}

fn cancel_bottom_hide(state: &Rc<RefCell<ViewerState>>) {
    if let Some(id) = state.borrow_mut().bottom_hide_source.take() {
        id.remove();
    }
}

/// Show the OSD pill for `OSD_PULSE_MS` on file-change / zoom. The pill
/// is bottom-centered above the strip area so it's visible without any
/// chrome taking over.
fn pulse_osd(state: &Rc<RefCell<ViewerState>>) {
    {
        let s = state.borrow();
        if s.osd_label.label().is_empty() {
            return;
        }
        s.osd_revealer.set_reveal_child(true);
    }
    cancel_osd_hide(state);
    if state.borrow().ui_pinned {
        return;
    }
    let clone = state.clone();
    let id =
        glib::timeout_add_local_once(std::time::Duration::from_millis(OSD_PULSE_MS), move || {
            hide_osd(&clone)
        });
    state.borrow_mut().osd_hide_source = Some(id);
}

fn hide_osd(state: &Rc<RefCell<ViewerState>>) {
    let mut s = state.borrow_mut();
    s.osd_hide_source = None;
    s.osd_revealer.set_reveal_child(false);
}

fn cancel_osd_hide(state: &Rc<RefCell<ViewerState>>) {
    if let Some(id) = state.borrow_mut().osd_hide_source.take() {
        id.remove();
    }
}

fn schedule_cursor_hide(state: &Rc<RefCell<ViewerState>>) {
    let s = state.borrow();
    if s.ui_pinned {
        return;
    }
    // AdwDialog (incluindo AdwAboutDialog) a partir da libadwaita 1.5 é
    // um modal in-window — compartilha a surface com o viewer. Se o
    // timer disparar enquanto o diálogo estiver aberto, o cursor sumiria
    // no próprio diálogo (o user não consegue clicar em "Detalhes" etc).
    // Pular a agenda enquanto houver diálogo visível na janela.
    if s.window.visible_dialog().is_some() {
        return;
    }
    drop(s);
    let clone = state.clone();
    let id =
        glib::timeout_add_local_once(std::time::Duration::from_millis(CURSOR_HIDE_MS), move || {
            // Double-check na hora do disparo: user pode ter aberto um
            // diálogo nos últimos CURSOR_HIDE_MS ms.
            let s = clone.borrow_mut();
            if s.window.visible_dialog().is_some() {
                drop(s);
                clone.borrow_mut().cursor_hide_source = None;
                return;
            }
            let cursor = gdk::Cursor::from_name("none", None);
            s.window.set_cursor(cursor.as_ref());
            drop(s);
            clone.borrow_mut().cursor_hide_source = None;
        });
    state.borrow_mut().cursor_hide_source = Some(id);
}

fn cancel_cursor_hide(state: &Rc<RefCell<ViewerState>>) {
    if let Some(id) = state.borrow_mut().cursor_hide_source.take() {
        id.remove();
    }
}

/// Flip the accessibility pin: when pinned, every chrome element stays
/// visible and no timer fires. When unpinned, we re-enter the natural
/// edge-triggered flow.
fn toggle_ui_pin(state: &Rc<RefCell<ViewerState>>) {
    let pinned = {
        let mut s = state.borrow_mut();
        s.ui_pinned = !s.ui_pinned;
        s.ui_pinned
    };
    if pinned {
        cancel_top_hide(state);
        cancel_bottom_hide(state);
        cancel_osd_hide(state);
        cancel_cursor_hide(state);
        let s = state.borrow();
        s.toolbar_view.set_reveal_top_bars(true);
        if s.film_strip_buttons.len() >= 2 {
            s.strip_revealer.set_reveal_child(true);
        }
        if !s.osd_label.label().is_empty() {
            s.osd_revealer.set_reveal_child(true);
        }
        s.window.set_cursor(None);
    } else {
        // Collapse back to the clean default. Upcoming motion events will
        // re-reveal edges on demand.
        let s = state.borrow();
        s.toolbar_view.set_reveal_top_bars(false);
        s.strip_revealer.set_reveal_child(false);
        s.osd_revealer.set_reveal_child(false);
        drop(s);
        schedule_cursor_hide(state);
    }
}

fn handle_key(
    state: &Rc<RefCell<ViewerState>>,
    key: gdk::Key,
    _modifier: gdk::ModifierType,
) -> glib::Propagation {
    use gdk::Key;

    match key {
        Key::Escape => {
            state.borrow().window.close();
            return glib::Propagation::Stop;
        }
        Key::F11 => {
            let win = state.borrow().window.clone();
            win.set_fullscreened(!win.is_fullscreen());
            return glib::Propagation::Stop;
        }
        Key::h | Key::H => {
            toggle_ui_pin(state);
            return glib::Propagation::Stop;
        }
        Key::t | Key::T => {
            toggle_theater(state);
            return glib::Propagation::Stop;
        }
        Key::f | Key::F => {
            toggle_film_strip(state);
            return glib::Propagation::Stop;
        }
        Key::plus | Key::equal | Key::KP_Add => {
            zoom_in(state);
            pulse_osd(state);
            return glib::Propagation::Stop;
        }
        Key::minus | Key::KP_Subtract => {
            zoom_out(state);
            pulse_osd(state);
            return glib::Propagation::Stop;
        }
        Key::_0 | Key::KP_0 => {
            set_zoom(state, ZoomState::Fit);
            pulse_osd(state);
            return glib::Propagation::Stop;
        }
        Key::_1 | Key::KP_1 => {
            set_zoom(state, ZoomState::Scale(1.0));
            pulse_osd(state);
            return glib::Propagation::Stop;
        }
        Key::space => {
            toggle_slideshow(state);
            return glib::Propagation::Stop;
        }
        Key::g | Key::G => {
            toggle_histogram(state);
            return glib::Propagation::Stop;
        }
        _ => {}
    }

    let total = state.borrow().files.len();
    if total == 0 {
        return glib::Propagation::Proceed;
    }

    let new_idx = match key {
        Key::Right | Key::Page_Down => Some((state.borrow().idx + 1) % total),
        Key::Left | Key::Page_Up | Key::BackSpace => {
            let cur = state.borrow().idx;
            Some(if cur == 0 { total - 1 } else { cur - 1 })
        }
        Key::Home => Some(0),
        Key::End => Some(total - 1),
        _ => None,
    };

    match new_idx {
        Some(idx) => {
            {
                let mut s = state.borrow_mut();
                s.idx = idx;
                // Reset to Fit on every file switch so each new image
                // shows whole rather than at the previous file's zoom.
                s.zoom = ZoomState::Fit;
            }
            update_display(state);
            pulse_osd(state);
            glib::Propagation::Stop
        }
        None => glib::Propagation::Proceed,
    }
}

/// Avança o índice em +1 (com wrap). Reseta zoom pra Fit como a nav
/// normal faz, pra cada imagem aparecer inteira no slideshow.
fn advance_one(state: &Rc<RefCell<ViewerState>>) {
    let total = state.borrow().files.len();
    if total < 2 {
        return;
    }
    {
        let mut s = state.borrow_mut();
        s.idx = (s.idx + 1) % total;
        s.zoom = ZoomState::Fit;
    }
    update_display(state);
    pulse_osd(state);
}

/// Alterna o slideshow (Space). Ignora quando só há um arquivo. Enquanto
/// ativo, um timer de `SLIDESHOW_INTERVAL_S` avança pra próxima imagem;
/// uma nova pulsação no OSD confirma cada tick.
fn toggle_slideshow(state: &Rc<RefCell<ViewerState>>) {
    let total = state.borrow().files.len();
    if total < 2 {
        return;
    }
    let active = state.borrow().slideshow_source.is_some();
    if active {
        if let Some(id) = state.borrow_mut().slideshow_source.take() {
            id.remove();
        }
        let s = state.borrow();
        s.osd_label.set_text("Slideshow: pausado");
        drop(s);
        pulse_osd(state);
        return;
    }
    let clone = state.clone();
    let id = glib::timeout_add_seconds_local(SLIDESHOW_INTERVAL_S, move || {
        // Se a janela fechou entre ticks, encerra — evita mexer em widgets
        // já removidos do tree.
        {
            let s = clone.borrow();
            if !s.window.is_visible() || s.files.is_empty() {
                drop(s);
                clone.borrow_mut().slideshow_source = None;
                return glib::ControlFlow::Break;
            }
        }
        advance_one(&clone);
        glib::ControlFlow::Continue
    });
    state.borrow_mut().slideshow_source = Some(id);
    let s = state.borrow();
    s.osd_label.set_text(&format!("Slideshow: {}s", SLIDESHOW_INTERVAL_S));
    drop(s);
    pulse_osd(state);
}

/// Alterna o overlay de histograma RGB (tecla G). Quando aparece, pulsa
/// o OSD pra confirmar visualmente — o quadrante superior-direito é
/// discreto e sem o pulso o user pode não perceber que ligou.
fn toggle_histogram(state: &Rc<RefCell<ViewerState>>) {
    let now_visible = {
        let s = state.borrow();
        let v = !s.histogram_area.is_visible();
        s.histogram_area.set_visible(v);
        if v {
            s.histogram_area.queue_draw();
        }
        v
    };
    {
        let s = state.borrow();
        s.osd_label.set_text(if now_visible {
            "Histograma: ligado"
        } else {
            "Histograma: desligado"
        });
    }
    pulse_osd(state);
}

/// Renderiza o histograma RGB sobre um fundo translúcido. Normaliza cada
/// canal pelo seu próprio máximo pra que canais muito desbalanceados
/// (ex: foto bem escura) ainda mostrem forma. Bins de 256 são quantizados
/// pra largura do widget usando acúmulo por bucket — assim larguras
/// menores que 256 não perdem picos isolados.
fn draw_histogram(state: &Rc<RefCell<ViewerState>>, cr: &gtk::cairo::Context, w: i32, h: i32) {
    let hist = state.borrow().histogram;
    let w = w as f64;
    let h = h as f64;

    // Fundo — preto translúcido + borda sutil, legível sobre qualquer foto.
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.55);
    cr.rectangle(0.0, 0.0, w, h);
    let _ = cr.fill();
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.15);
    cr.set_line_width(1.0);
    cr.rectangle(0.5, 0.5, w - 1.0, h - 1.0);
    let _ = cr.stroke();

    // Colapsa os 256 bins pra largura do widget. Se w < 256, cada coluna
    // representa um intervalo; somamos os bins pra não perder picos.
    let cols = (w as usize).max(1);
    let mut bars = vec![[0u32; 3]; cols];
    for (i, channel) in hist.chunks(256).enumerate() {
        for (bin, &v) in channel.iter().enumerate() {
            let col = (bin * cols) / 256;
            bars[col][i] = bars[col][i].saturating_add(v);
        }
    }
    let max_per_channel: [u32; 3] = [
        bars.iter().map(|b| b[0]).max().unwrap_or(1).max(1),
        bars.iter().map(|b| b[1]).max().unwrap_or(1).max(1),
        bars.iter().map(|b| b[2]).max().unwrap_or(1).max(1),
    ];
    let inner_h = h - 2.0;
    // Canais em ordem B, G, R — fica mais parecido com Photoshop (R mais
    // destacado no topo dos overlaps aditivos).
    let channels: [(usize, (f64, f64, f64)); 3] =
        [(2, (0.20, 0.55, 1.00)), (1, (0.30, 0.90, 0.40)), (0, (1.00, 0.30, 0.30))];
    for (idx, (r, g, b)) in channels {
        cr.set_source_rgba(r, g, b, 0.75);
        for (i, bucket) in bars.iter().enumerate() {
            let v = bucket[idx] as f64 / max_per_channel[idx] as f64;
            let bar_h = v * inner_h;
            cr.rectangle((i as f64) + 0.5, h - 1.0 - bar_h, 1.0, bar_h);
        }
        let _ = cr.fill();
    }
}

/// Aplica uma edição in-memory. O pipeline é: acumula op, reabre o
/// arquivo do disco, reaplica todas as ops (inclusive as anteriores —
/// barato mesmo com fotos grandes), produz a textura nova. Reaplicar
/// tudo em vez de guardar a `DynamicImage` editada entre turnos evita
/// drift cumulativo e simplifica o undo futuro (basta `pending_ops.pop`).
fn apply_edit(state: &Rc<RefCell<ViewerState>>, op: EditOp) {
    let path = {
        let s = state.borrow();
        if s.files.is_empty() {
            return;
        }
        s.files[s.idx].clone()
    };
    state.borrow_mut().pending_ops.push(op);
    rebuild_edited_texture(state, &path);
    refresh_title(state);
    sync_save_action(state);
    pulse_osd(state);
}

/// Reaplica `pending_ops` sobre a imagem no disco e substitui a textura.
/// Se não houver ops, equivale a recarregar o arquivo via `load_texture`.
/// Falhas de decode logam e não mexem no paintable (o user vê a imagem
/// anterior — melhor do que uma tela preta).
fn rebuild_edited_texture(state: &Rc<RefCell<ViewerState>>, path: &Path) {
    let ops = state.borrow().pending_ops.clone();
    if ops.is_empty() {
        // Caminho rápido: sem edições, usa a versão crua do disco direto.
        if let Ok((tex, nat, hist)) = load_texture(path) {
            let mut s = state.borrow_mut();
            s.natural = nat;
            s.histogram = hist;
            s.picture.set_paintable(Some(&tex));
            apply_zoom(&s.picture, s.zoom, nat);
            s.histogram_area.queue_draw();
        }
        return;
    }
    let img = match image::open(path) {
        Ok(i) => i,
        Err(e) => {
            tracing::error!(?path, error = %e, "edit: falha ao reabrir pra edição");
            return;
        }
    };
    let edited = ops.iter().fold(img, |acc, op| apply_op_to_image(acc, *op));
    let rgba = edited.to_rgba8();
    let (w, h) = rgba.dimensions();
    let raw = rgba.into_raw();
    let histogram = compute_histogram(&raw);
    let stride = (w as usize) * 4;
    let bytes = glib::Bytes::from_owned(raw);
    let tex =
        gdk::MemoryTexture::new(w as i32, h as i32, gdk::MemoryFormat::R8g8b8a8, &bytes, stride);
    let paintable: gdk::Texture = tex.upcast();
    let mut s = state.borrow_mut();
    s.natural = (w, h);
    s.histogram = histogram;
    s.picture.set_paintable(Some(&paintable));
    apply_zoom(&s.picture, s.zoom, (w, h));
    s.histogram_area.queue_draw();
}

fn apply_op_to_image(img: image::DynamicImage, op: EditOp) -> image::DynamicImage {
    match op {
        EditOp::RotateCw => img.rotate90(),
        EditOp::RotateCcw => img.rotate270(),
        EditOp::Rotate180 => img.rotate180(),
        EditOp::FlipH => img.fliph(),
        EditOp::FlipV => img.flipv(),
    }
}

/// Liga/desliga a action `win.save-edits` pra o item de menu e o acelerador
/// Ctrl+S ficarem grey quando não houver nada pra salvar. Complementa o
/// `•` no título como pista visual do estado "dirty".
fn sync_save_action(state: &Rc<RefCell<ViewerState>>) {
    let s = state.borrow();
    let has_ops = !s.pending_ops.is_empty();
    if let Some(action) = s.window.lookup_action("save-edits") {
        if let Some(simple) = action.downcast_ref::<gio::SimpleAction>() {
            simple.set_enabled(has_ops);
        }
    }
}

/// Salva as edições pendentes no próprio arquivo. Confirma antes — em
/// JPEG o re-encode perde qualidade e o user precisa ciência disso.
fn save_edits(state: &Rc<RefCell<ViewerState>>) {
    let (window, path, ops) = {
        let s = state.borrow();
        if s.pending_ops.is_empty() || s.files.is_empty() {
            return;
        }
        (s.window.clone(), s.files[s.idx].clone(), s.pending_ops.clone())
    };
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    let is_jpeg = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| matches!(s.to_ascii_lowercase().as_str(), "jpg" | "jpeg"))
        .unwrap_or(false);
    let body = if is_jpeg {
        format!(
            "\"{name}\" será sobrescrito. JPEG é recodificado a cada \
             gravação — isso pode degradar a qualidade um pouco."
        )
    } else {
        format!("\"{name}\" será sobrescrito.")
    };
    let alert = adw::AlertDialog::builder().heading("Salvar alterações?").body(body).build();
    alert.add_response("cancel", "Cancelar");
    alert.add_response("save", "Salvar");
    alert.set_response_appearance("save", adw::ResponseAppearance::Suggested);
    alert.set_default_response(Some("save"));
    alert.set_close_response("cancel");

    let state = state.clone();
    alert.connect_response(Some("save"), move |_, _| match write_edits_to_disk(&path, &ops) {
        Ok(()) => {
            state.borrow_mut().pending_ops.clear();
            refresh_title(&state);
            sync_save_action(&state);
            let s = state.borrow();
            s.osd_label.set_text("Salvo");
            drop(s);
            pulse_osd(&state);
        }
        Err(e) => {
            tracing::error!(?path, error = %e, "save: falha ao gravar edições");
            let s = state.borrow();
            s.osd_label.set_text("Falha ao salvar");
            drop(s);
            pulse_osd(&state);
        }
    });
    alert.present(Some(&window));
}

/// Grava uma versão editada em cima do arquivo original. Re-decoda pro
/// estado atual das ops e usa `DynamicImage::save` — que escolhe o
/// encoder pela extensão. Nada de pipeline bigimage-core aqui: evita
/// arrastar política de sufixo/overwrite pra dentro do caminho do viewer.
fn write_edits_to_disk(path: &Path, ops: &[EditOp]) -> Result<(), Box<dyn std::error::Error>> {
    let img = image::open(path)?;
    let edited = ops.iter().fold(img, |acc, op| apply_op_to_image(acc, *op));
    edited.save(path)?;
    Ok(())
}

fn zoom_in(state: &Rc<RefCell<ViewerState>>) {
    adjust_zoom(state, ZOOM_STEP, None);
}

fn zoom_out(state: &Rc<RefCell<ViewerState>>) {
    adjust_zoom(state, 1.0 / ZOOM_STEP, None);
}

/// Rescale by `factor`, optionally anchoring the change at a cursor point
/// (in scroller-local coords) so the image pixel under the cursor stays
/// still. With `cursor = None` we just rescale and let the scroll position
/// clamp itself — fine for keyboard shortcuts.
fn adjust_zoom(state: &Rc<RefCell<ViewerState>>, factor: f64, cursor: Option<(f64, f64)>) {
    let (natural, viewport, current, hx, vy) = {
        let s = state.borrow();
        (
            s.natural,
            (s.scroller.width(), s.scroller.height()),
            s.zoom,
            s.scroller.hadjustment().value(),
            s.scroller.vadjustment().value(),
        )
    };
    if natural.0 == 0 || natural.1 == 0 {
        return;
    }
    let old_scale = effective_scale(current, natural, viewport);
    let new_scale = (old_scale * factor).clamp(MIN_ZOOM, MAX_ZOOM);
    if (new_scale - old_scale).abs() < 1e-6 {
        return;
    }

    set_zoom(state, ZoomState::Scale(new_scale));

    // Re-anchor the scroll so the image pixel under `cursor` stays still.
    // The Picture uses halign=Center/valign=Center, so when it's smaller
    // than the viewport it's padded equally on both sides — that padding
    // has to enter the formula or the anchor point drifts. We compute the
    // old/new offsets from pre/post-zoom widget sizes vs. viewport size.
    if let Some((cx, cy)) = cursor {
        let (target_hx, max_hx, new_pic_w) =
            anchor_axis(cx, hx, old_scale, new_scale, f64::from(natural.0), f64::from(viewport.0));
        let (target_vy, max_vy, new_pic_h) =
            anchor_axis(cy, vy, old_scale, new_scale, f64::from(natural.1), f64::from(viewport.1));
        // The scroller's adjustments update their `upper` only after the
        // layout pass that follows our set_width_request(). A single
        // idle_add_local_once fires BEFORE that pass on many GTK builds,
        // so set_value() gets clamped to the stale upper (0 in Fit mode)
        // and the anchor snaps to the top-left. Poll up to a handful of
        // idle ticks, apply when upper catches up, and bail defensively.
        let state = state.clone();
        let attempts = Rc::new(Cell::new(0u32));
        glib::idle_add_local(move || {
            let s = state.borrow();
            let hadj = s.scroller.hadjustment();
            let vadj = s.scroller.vadjustment();
            let ready_x = max_hx == 0.0 || hadj.upper() >= new_pic_w - 0.5;
            let ready_y = max_vy == 0.0 || vadj.upper() >= new_pic_h - 0.5;
            let tries = attempts.get();
            if ready_x && ready_y {
                hadj.set_value(target_hx);
                vadj.set_value(target_vy);
                return glib::ControlFlow::Break;
            }
            if tries >= 10 {
                // Give up gracefully — apply whatever the adjustment will
                // accept so the anchor at least lands in-range. Better
                // than looping forever on a misbehaving driver.
                hadj.set_value(target_hx);
                vadj.set_value(target_vy);
                return glib::ControlFlow::Break;
            }
            attempts.set(tries + 1);
            glib::ControlFlow::Continue
        });
    }
}

/// Math-only core of cursor-anchored zoom on one axis. Returns
/// `(target_adj, max_adj, new_pic_size)`:
///  * `target_adj` — the adjustment value (pre-clamp-to-layout) that keeps
///    the image pixel under the cursor in place after scaling. Already
///    clamped to the post-zoom content range so it can be applied directly.
///  * `max_adj` — upper bound of the adjustment (`pic - viewport`, floored
///    at 0), returned so the caller can detect "layout not ready yet".
///  * `new_pic_size` — post-zoom picture size, same rationale.
///
/// Coordinate model (see `adjust_zoom` doc): the Picture is centered in
/// the viewport when smaller than it, so we fold the letterbox offset into
/// the projection. With `cursor=None` the caller skips this entirely and
/// lets the adjustment clamp naturally.
fn anchor_axis(
    cursor: f64,
    adj: f64,
    old_scale: f64,
    new_scale: f64,
    natural: f64,
    viewport: f64,
) -> (f64, f64, f64) {
    let ratio = new_scale / old_scale;
    let old_pic = natural * old_scale;
    let new_pic = natural * new_scale;
    let old_off = ((viewport - old_pic) / 2.0).max(0.0);
    let new_off = ((viewport - new_pic) / 2.0).max(0.0);
    let new_adj = ratio * (cursor + adj - old_off) - cursor + new_off;
    let max_adj = (new_pic - viewport).max(0.0);
    (new_adj.clamp(0.0, max_adj), max_adj, new_pic)
}

fn set_zoom(state: &Rc<RefCell<ViewerState>>, zoom: ZoomState) {
    {
        let mut s = state.borrow_mut();
        s.zoom = zoom;
        apply_zoom(&s.picture, zoom, s.natural);
    }
    refresh_title(state);
}

fn apply_zoom(picture: &gtk::Picture, zoom: ZoomState, natural: (u32, u32)) {
    match zoom {
        ZoomState::Fit => {
            picture.set_content_fit(gtk::ContentFit::Contain);
            picture.set_width_request(-1);
            picture.set_height_request(-1);
        }
        ZoomState::Scale(scale) => {
            // Always Contain: ContentFit::Fill stretches to the allocated
            // size (which the ScrolledWindow may grow past our size_request
            // for small-scale images), producing visible distortion. Contain
            // keeps the aspect ratio even if the allocation overshoots.
            picture.set_content_fit(gtk::ContentFit::Contain);
            let w = ((natural.0 as f64) * scale).round().max(1.0) as i32;
            let h = ((natural.1 as f64) * scale).round().max(1.0) as i32;
            picture.set_width_request(w);
            picture.set_height_request(h);
        }
    }
}

/// Effective on-screen scale factor — used to anchor zoom-in from `Fit` at
/// the current visual size, so the first click doesn't teleport the image.
fn effective_scale(zoom: ZoomState, natural: (u32, u32), viewport: (i32, i32)) -> f64 {
    match zoom {
        ZoomState::Scale(s) => s,
        ZoomState::Fit => {
            if natural.0 == 0 || natural.1 == 0 || viewport.0 <= 0 || viewport.1 <= 0 {
                return 1.0;
            }
            let sx = viewport.0 as f64 / natural.0 as f64;
            let sy = viewport.1 as f64 / natural.1 as f64;
            sx.min(sy).min(1.0)
        }
    }
}

fn update_display(state: &Rc<RefCell<ViewerState>>) {
    let path = {
        let s = state.borrow();
        s.files[s.idx].clone()
    };
    // Trocar de arquivo descarta edições pendentes — o estilo Loupe
    // clássico pediria confirmação; aqui priorizamos velocidade de nav
    // (arrow-spam, slideshow) e sinalizamos via OSD quando algo foi
    // descartado.
    let had_pending = !state.borrow().pending_ops.is_empty();
    state.borrow_mut().pending_ops.clear();
    sync_save_action(state);
    match load_texture(&path) {
        Ok((tex, nat, hist)) => {
            let mut s = state.borrow_mut();
            s.natural = nat;
            s.histogram = hist;
            s.picture.set_paintable(Some(&tex));
            apply_zoom(&s.picture, s.zoom, nat);
            s.histogram_area.queue_draw();
        }
        Err(e) => {
            tracing::error!(?path, error = %e, "falha ao carregar imagem");
            let s = state.borrow();
            s.picture.set_paintable(gdk::Paintable::NONE);
        }
    }
    refresh_title(state);
    if had_pending {
        let s = state.borrow();
        s.osd_label.set_text("Edições descartadas");
        drop(s);
        pulse_osd(state);
    }
}

/// Constrói o botão-menu hamburger. Itens seguem a convenção Loupe
/// (viewer nativo do GNOME): abrir, copiar, imprimir, metadados, lixeira,
/// atalhos, sobre — separados por seções pra agrupar por semântica.
/// Ações ficam em `win.<nome>` (registradas em [`install_viewer_actions`]).
fn build_header_menu_button(state: &Rc<RefCell<ViewerState>>) -> gtk::MenuButton {
    let menu = gio::Menu::new();

    let s_file = gio::Menu::new();
    s_file.append(Some("Abrir imagens…"), Some("win.open"));
    s_file.append(Some("Copiar imagem"), Some("win.copy"));
    menu.append_section(None, &s_file);

    let s_actions = gio::Menu::new();
    s_actions.append(Some("Imprimir…"), Some("win.print"));
    menu.append_section(None, &s_actions);

    // Edição in-memory estilo Loupe: giros/espelhos aplicados na hora;
    // só vão pra disco quando o user pedir Salvar (Ctrl+S).
    let s_edit = gio::Menu::new();
    s_edit.append(Some("Girar à direita"), Some("win.rotate-cw"));
    s_edit.append(Some("Girar à esquerda"), Some("win.rotate-ccw"));
    s_edit.append(Some("Girar 180°"), Some("win.rotate-180"));
    s_edit.append(Some("Espelhar horizontal"), Some("win.flip-h"));
    s_edit.append(Some("Espelhar vertical"), Some("win.flip-v"));
    s_edit.append(Some("Recortar…"), Some("win.crop"));
    menu.append_section(None, &s_edit);

    // IA: cada item dispara um subprocesso `bigiris --dialog=…` (mesmo
    // motivo do crop: cada diálogo monta sua própria adw::Application).
    let s_ai = gio::Menu::new();
    s_ai.append(Some("Remover fundo (IA)…"), Some("win.remove-bg"));
    s_ai.append(Some("Aumentar resolução…"), Some("win.upscale"));
    menu.append_section(None, &s_ai);

    let s_save = gio::Menu::new();
    s_save.append(Some("Salvar alterações"), Some("win.save-edits"));
    menu.append_section(None, &s_save);

    let s_delete = gio::Menu::new();
    s_delete.append(Some("Mover para a lixeira"), Some("win.trash"));
    menu.append_section(None, &s_delete);

    let s_help = gio::Menu::new();
    s_help.append(Some("Atalhos do teclado"), Some("win.shortcuts"));
    s_help.append(Some("Sobre o BigIris"), Some("app.about"));
    menu.append_section(None, &s_help);

    let btn = gtk::MenuButton::new();
    btn.set_icon_name("open-menu-symbolic");
    btn.set_tooltip_text(Some("Menu principal"));
    btn.set_menu_model(Some(&menu));
    // Fixa a header enquanto o popover está aberto — senão o timer de
    // auto-hide do ToolbarView dispara com o cursor já dentro do popover
    // e fecha o menu sozinho (flicker visível: abre e some no mesmo frame).
    {
        let state = state.clone();
        btn.connect_active_notify(move |b| {
            if b.is_active() {
                cancel_top_hide(&state);
                state.borrow().toolbar_view.set_reveal_top_bars(true);
            } else {
                reveal_top(&state);
            }
        });
    }
    btn
}

/// Registra as actions `win.*` que alimentam o menu hamburger e os
/// quick-buttons do header, mais os acelerators globais. Tudo fecha
/// sobre `state` pra acessar a imagem corrente.
fn install_viewer_actions(
    app: &adw::Application,
    window: &adw::ApplicationWindow,
    state: &Rc<RefCell<ViewerState>>,
) {
    // Sobre — já existia, reinstalamos aqui pra ficar tudo num lugar.
    let about = gio::SimpleAction::new("about", None);
    {
        let window = window.clone();
        about.connect_activate(move |_, _| present_about_dialog(&window));
    }
    app.add_action(&about);

    // win.fullscreen — alterna F11; quick-button do header aciona a mesma.
    let fullscreen = gio::SimpleAction::new("fullscreen", None);
    {
        let window = window.clone();
        fullscreen.connect_activate(move |_, _| {
            window.set_fullscreened(!window.is_fullscreen());
        });
    }
    window.add_action(&fullscreen);

    // win.open — FileDialog → spawna nova instância do bigiris nos arquivos
    // escolhidos. Separar em nova janela mantém o histórico da atual.
    let open = gio::SimpleAction::new("open", None);
    {
        let window = window.clone();
        open.connect_activate(move |_, _| open_file_chooser(&window));
    }
    window.add_action(&open);

    // Actions dependentes de imagem carregada ficam grey na dashboard —
    // senão o menu mostra opção clicável que não faz nada, e itens de
    // ação inválida chegam a fechar o popover ao hover no GTK4.
    let has_files = !state.borrow().files.is_empty();

    // win.copy — copia a textura atual pro clipboard. Qualquer app (Slack,
    // Ctrl+V num chat) consegue colar.
    let copy = gio::SimpleAction::new("copy", None);
    copy.set_enabled(has_files);
    {
        let state = state.clone();
        copy.connect_activate(move |_, _| copy_current_to_clipboard(&state));
    }
    window.add_action(&copy);

    // win.rotate-{cw,ccw,180}, win.flip-{h,v} — edições in-memory. Acumulam
    // em `pending_ops`, atualizam textura na hora e marcam o título com `•`.
    let edits: [(&str, EditOp, &[&str]); 5] = [
        ("rotate-cw", EditOp::RotateCw, &["R"]),
        ("rotate-ccw", EditOp::RotateCcw, &["<Shift>R"]),
        ("rotate-180", EditOp::Rotate180, &[]),
        ("flip-h", EditOp::FlipH, &[]),
        ("flip-v", EditOp::FlipV, &[]),
    ];
    for (name, op, accels) in edits {
        let action = gio::SimpleAction::new(name, None);
        action.set_enabled(has_files);
        {
            let state = state.clone();
            action.connect_activate(move |_, _| apply_edit(&state, op));
        }
        window.add_action(&action);
        if !accels.is_empty() {
            app.set_accels_for_action(&format!("win.{name}"), accels);
        }
    }

    // win.save-edits — grava as `pending_ops` no arquivo. Sai grey quando
    // não há nada pra salvar (controlado por `sync_save_action`).
    let save_edits_action = gio::SimpleAction::new("save-edits", None);
    save_edits_action.set_enabled(false);
    {
        let state = state.clone();
        save_edits_action.connect_activate(move |_, _| save_edits(&state));
    }
    window.add_action(&save_edits_action);
    app.set_accels_for_action("win.save-edits", &["<Ctrl>S"]);

    // win.print — gtk::PrintOperation + cairo, escala a imagem pra página
    // preservando aspecto (Contain).
    let print = gio::SimpleAction::new("print", None);
    print.set_enabled(has_files);
    {
        let state = state.clone();
        print.connect_activate(move |_, _| print_current(&state));
    }
    window.add_action(&print);

    // win.info — reabre o dialog de metadados no arquivo corrente.
    let info = gio::SimpleAction::new("info", None);
    info.set_enabled(has_files);
    {
        let state = state.clone();
        info.connect_activate(move |_, _| show_metadata_for_current(&state));
    }
    window.add_action(&info);

    // win.crop — abre o diálogo de recorte na imagem atual via subprocesso.
    // Um processo separado porque `run_crop_dialog` monta sua própria
    // `adw::Application`, e rodar duas no mesmo processo é conflito certo.
    let crop = gio::SimpleAction::new("crop", None);
    crop.set_enabled(has_files);
    {
        let state = state.clone();
        crop.connect_activate(move |_, _| {
            let path = {
                let s = state.borrow();
                if s.files.is_empty() {
                    return;
                }
                s.files[s.idx].clone()
            };
            if let Ok(exe) = std::env::current_exe() {
                let _ = std::process::Command::new(exe).arg("--dialog=crop").arg(path).spawn();
            }
        });
    }
    window.add_action(&crop);

    // win.remove-bg / win.upscale — mesmo padrão do crop: subprocesso
    // dedicado para cada diálogo Prisma na imagem corrente.
    for (name, dialog) in [("remove-bg", "remove-bg"), ("upscale", "upscale")] {
        let action = gio::SimpleAction::new(name, None);
        action.set_enabled(has_files);
        let state = state.clone();
        let dialog = dialog.to_string();
        action.connect_activate(move |_, _| {
            let path = {
                let s = state.borrow();
                if s.files.is_empty() {
                    return;
                }
                s.files[s.idx].clone()
            };
            if let Ok(exe) = std::env::current_exe() {
                let _ = std::process::Command::new(exe)
                    .arg(format!("--dialog={dialog}"))
                    .arg(path)
                    .spawn();
            }
        });
        window.add_action(&action);
    }

    // win.trash — confirma e joga o arquivo na lixeira do XDG. Como o
    // state guarda files como Rc<Vec>, depois de deletar fechamos a
    // janela em vez de tentar mutar a lista (o usuário reabre o próximo).
    let trash = gio::SimpleAction::new("trash", None);
    trash.set_enabled(has_files);
    {
        let state = state.clone();
        trash.connect_activate(move |_, _| trash_current(&state));
    }
    window.add_action(&trash);

    // Fullscreen só faz sentido com imagem; dashboard tem flow diferente.
    let fullscreen_action = window.lookup_action("fullscreen").and_downcast::<gio::SimpleAction>();
    if let Some(action) = fullscreen_action {
        action.set_enabled(has_files);
    }

    // win.shortcuts — AdwAlertDialog com cheatsheet. Mais leve e
    // legível que gtk::ShortcutsWindow, e combina com o resto do visual.
    let shortcuts = gio::SimpleAction::new("shortcuts", None);
    {
        let window = window.clone();
        shortcuts.connect_activate(move |_, _| present_shortcuts_dialog(&window));
    }
    window.add_action(&shortcuts);

    app.set_accels_for_action("win.open", &["<Ctrl>o"]);
    app.set_accels_for_action("win.copy", &["<Ctrl>c"]);
    app.set_accels_for_action("win.print", &["<Ctrl>p"]);
    app.set_accels_for_action("win.info", &["<Ctrl>i"]);
    app.set_accels_for_action("win.crop", &["<Ctrl>r"]);
    app.set_accels_for_action("win.remove-bg", &["<Ctrl>b"]);
    app.set_accels_for_action("win.upscale", &["<Ctrl>u"]);
    app.set_accels_for_action("win.trash", &["Delete"]);
    app.set_accels_for_action("win.shortcuts", &["<Ctrl>question"]);
    app.set_accels_for_action("win.fullscreen", &["F11"]);
    app.set_accels_for_action("app.about", &["F1"]);
}

/// Apresenta um FileDialog de seleção múltipla com filtro de imagens.
/// Abre cada seleção numa nova janela via `std::process::Command` pra
/// preservar o histórico da janela atual (e porque a `files` no state
/// é Rc<Vec<_>> imutável).
fn open_file_chooser(window: &adw::ApplicationWindow) {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("Imagens"));
    filter.add_mime_type("image/*");
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);

    let dialog =
        gtk::FileDialog::builder().title("Abrir imagens").modal(true).filters(&filters).build();

    dialog.open_multiple(Some(window), gio::Cancellable::NONE, move |result| {
        let Ok(list) = result else { return };
        let mut paths: Vec<String> = Vec::new();
        for i in 0..list.n_items() {
            if let Some(file) = list.item(i).and_downcast::<gio::File>() {
                if let Some(p) = file.path() {
                    paths.push(p.to_string_lossy().into_owned());
                }
            }
        }
        if paths.is_empty() {
            return;
        }
        let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("bigiris"));
        let _ = std::process::Command::new(exe).args(&paths).spawn();
    });
}

/// Copia a imagem corrente (pixels, não o path) pro clipboard do sistema.
/// Usa `gdk::MemoryTexture` — qualquer consumidor que aceite imagens
/// (paste em chat, editor de imagem) recebe a textura.
fn copy_current_to_clipboard(state: &Rc<RefCell<ViewerState>>) {
    let (window, path) = {
        let s = state.borrow();
        if s.files.is_empty() {
            return;
        }
        (s.window.clone(), s.files[s.idx].clone())
    };
    let Ok(img) = image::open(&path) else {
        tracing::warn!(?path, "copy: falha ao decodificar");
        return;
    };
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    let stride = (w as usize) * 4;
    let bytes = glib::Bytes::from_owned(rgba.into_raw());
    let tex =
        gdk::MemoryTexture::new(w as i32, h as i32, gdk::MemoryFormat::R8g8b8a8, &bytes, stride);
    window.clipboard().set_texture(&tex);
}

/// Envia a imagem corrente pra impressora via gtk::PrintOperation.
/// Renderiza a imagem com cairo numa única página, escalada pra
/// caber com aspect ratio preservado (Contain).
fn print_current(state: &Rc<RefCell<ViewerState>>) {
    let (window, path) = {
        let s = state.borrow();
        if s.files.is_empty() {
            return;
        }
        (s.window.clone(), s.files[s.idx].clone())
    };
    let op = gtk::PrintOperation::new();
    op.set_n_pages(1);
    op.set_job_name(
        &path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "BigIris".to_string()),
    );

    op.connect_draw_page(move |_, ctx, _page_nr| {
        let Ok(img) = image::open(&path) else { return };
        let rgba = img.to_rgba8();
        let (iw, ih) = rgba.dimensions();

        // cairo::Format::Rgb24 e ARgb32 querem BGRA. Convertemos RGBA→BGRA
        // in-place (só swap dos canais R↔B).
        let mut bgra = rgba.into_raw();
        for px in bgra.chunks_exact_mut(4) {
            px.swap(0, 2);
        }
        let stride = iw as i32 * 4;
        let Ok(surface) = gtk::cairo::ImageSurface::create_for_data(
            bgra,
            gtk::cairo::Format::ARgb32,
            iw as i32,
            ih as i32,
            stride,
        ) else {
            return;
        };

        let cr = ctx.cairo_context();
        let pw = ctx.width();
        let ph = ctx.height();
        let scale = (pw / iw as f64).min(ph / ih as f64);
        let dx = (pw - iw as f64 * scale) / 2.0;
        let dy = (ph - ih as f64 * scale) / 2.0;
        cr.translate(dx, dy);
        cr.scale(scale, scale);
        let _ = cr.set_source_surface(&surface, 0.0, 0.0);
        let _ = cr.paint();
    });

    if let Err(e) = op.run(gtk::PrintOperationAction::PrintDialog, Some(&window)) {
        tracing::warn!(error = %e, "impressão cancelada/erro");
    }
}

/// Reusa o dialog de metadados existente com os arquivos do viewer.
/// Spawna nova janela (o dialog é uma AppWindow independente).
fn show_metadata_for_current(state: &Rc<RefCell<ViewerState>>) {
    // Se já existe uma janela aberta, traz pra frente em vez de empilhar.
    if let Some(existing) = state.borrow().props_dialog.clone() {
        existing.present();
        return;
    }
    let (window, path) = {
        let s = state.borrow();
        if s.files.is_empty() {
            return;
        }
        (s.window.clone(), s.files[s.idx].clone())
    };
    let Some(app) = window.application().and_downcast::<adw::Application>() else { return };
    let dialog = build_properties_window(&app, &path);
    // Quando o user fecha, soltamos a referência — próximo clique abre fresco.
    {
        let state = state.clone();
        dialog.connect_close_request(move |_| {
            state.borrow_mut().props_dialog = None;
            glib::Propagation::Proceed
        });
    }
    state.borrow_mut().props_dialog = Some(dialog.clone());
    dialog.present();
}

/// Confirma com AdwAlertDialog e move o arquivo corrente pra lixeira
/// via `gio::File::trash`. Depois fecha a janela — a `files` do state
/// é imutável (Rc<Vec>), então remover elemento exigiria refactor.
fn trash_current(state: &Rc<RefCell<ViewerState>>) {
    let (window, path) = {
        let s = state.borrow();
        if s.files.is_empty() {
            return;
        }
        (s.window.clone(), s.files[s.idx].clone())
    };
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    let alert = adw::AlertDialog::builder()
        .heading("Mover para a lixeira?")
        .body(format!(
            "\"{name}\" será movido para a lixeira do sistema. Você pode restaurá-lo depois."
        ))
        .build();
    alert.add_response("cancel", "Cancelar");
    alert.add_response("trash", "Mover");
    alert.set_response_appearance("trash", adw::ResponseAppearance::Destructive);
    alert.set_default_response(Some("cancel"));
    alert.set_close_response("cancel");

    let window_for_close = window.clone();
    alert.connect_response(Some("trash"), move |_, _| {
        let file = gio::File::for_path(&path);
        match file.trash(gio::Cancellable::NONE) {
            Ok(_) => {
                tracing::info!(?path, "trash ok");
                window_for_close.close();
            }
            Err(e) => tracing::warn!(?path, error = %e, "trash falhou"),
        }
    });
    alert.present(Some(&window));
}

/// Cheatsheet dos atalhos de teclado do viewer. AdwAlertDialog em vez de
/// gtk::ShortcutsWindow pra ficar consistente com o resto dos dialogs
/// do app (mesma estética, modal-in-window).
fn present_shortcuts_dialog(parent: &adw::ApplicationWindow) {
    parent.set_cursor(None);
    // Usa o widget nativo `gtk::ShortcutsWindow` — layout denso estilo
    // Loupe/GNOME com colunas, seções e chips de tecla renderizadas pelo
    // próprio tema. Bem mais limpo do que texto em AlertDialog.
    let shortcuts: [(&str, &[(&str, &str)]); 6] = [
        (
            "Navegação",
            &[
                ("Left Right", "Anterior / próximo"),
                ("Home End", "Primeiro / último"),
                ("space", "Slideshow (auto-avanço)"),
            ],
        ),
        (
            "Zoom",
            &[
                ("plus minus", "Aproximar / afastar"),
                ("0", "Ajustar à janela"),
                ("1", "Tamanho real (100%)"),
                ("scroll_wheel", "Zoom contínuo"),
            ],
        ),
        (
            "Janela",
            &[
                ("F11", "Tela cheia"),
                ("T", "Modo teatro"),
                ("F", "Fita de miniaturas"),
                ("G", "Histograma RGB"),
                ("H", "Fixar barras visíveis"),
                ("Escape", "Fechar"),
            ],
        ),
        (
            "Edição",
            &[
                ("R", "Girar à direita"),
                ("<Shift>R", "Girar à esquerda"),
                ("<ctrl>R", "Recortar…"),
                ("<ctrl>B", "Remover fundo (IA)…"),
                ("<ctrl>U", "Aumentar resolução…"),
                ("<ctrl>S", "Salvar alterações"),
            ],
        ),
        (
            "Arquivo",
            &[
                ("<ctrl>O", "Abrir imagens"),
                ("<ctrl>C", "Copiar imagem"),
                ("<ctrl>P", "Imprimir"),
                ("<ctrl>I", "Propriedades"),
                ("Delete", "Mover para a lixeira"),
            ],
        ),
        ("Ajuda", &[("<ctrl>question", "Este diálogo"), ("F1", "Sobre")]),
    ];
    let section = gtk::ShortcutsSection::builder().section_name("viewer").build();
    for (group_title, entries) in shortcuts {
        let group = gtk::ShortcutsGroup::builder().title(group_title).build();
        for (accel, desc) in entries {
            let sc = gtk::ShortcutsShortcut::builder().accelerator(*accel).title(*desc).build();
            group.append(&sc);
        }
        section.append(&group);
    }
    let window =
        gtk::ShortcutsWindow::builder().modal(true).transient_for(parent).child(&section).build();
    window.present();
}

/// AdwAboutDialog padrão do GNOME — mostra nome, ícone, versão, licença,
/// desenvolvedores e links de suporte. Acionado pelo item "Sobre" do
/// menu da header. Esse é o análogo ao bloco "Detalhes / Relatar / Créditos
/// / Aviso legal" do Big OCR PDF que o usuário mostrou nas screenshots.
fn present_about_dialog(parent: &adw::ApplicationWindow) {
    // Restaura o cursor antes de apresentar — o viewer pode tê-lo
    // escondido por idle; dentro do AdwAboutDialog (modal in-window)
    // precisamos que esteja visível pra user navegar nos Detalhes etc.
    parent.set_cursor(None);

    let about = adw::AboutDialog::builder()
        .application_name("BigIris")
        .application_icon(APP_ID)
        .version(env!("CARGO_PKG_VERSION"))
        .developer_name("Leonardo Athayde")
        .copyright("© 2026 Leonardo Athayde · Comunidade BigLinux")
        .license_type(gtk::License::Gpl30)
        .website("https://biglinux.com.br")
        .issue_url("https://github.com/xathay/bigiris/issues")
        .support_url("https://forum.biglinux.com.br")
        .developers(vec!["Leonardo Athayde".to_string()])
        .designers(vec!["Comunidade BigLinux".to_string()])
        .translator_credits("Leonardo Athayde <leoathayde@gmail.com>")
        .comments(
            "Visualizador e conversor de imagens do BigLinux. \
             Rápido, acessível por teclado e integrado ao gerenciador \
             de arquivos via clique-direito.",
        )
        .build();
    about.present(Some(parent));
}

/// Welcome dashboard shown when `bigiris` is launched without files. Sets
/// the tone for the app: explains what it does, lists the headline
/// capabilities, and gives a single obvious next step ("Abrir imagens").
/// Inspired by the Big OCR PDF welcome — makes the first-contact moment
/// feel intentional instead of the usual empty-state-with-icon.
fn build_dashboard(window: &adw::ApplicationWindow) -> gtk::Widget {
    let status = adw::StatusPage::builder()
        .icon_name("com.biglinux.Iris")
        .title("Bem-vindo ao BigIris")
        .description(
            "Visualizador e conversor de imagens do BigLinux.\n\
             Pensado para ser rápido, acessível por teclado e integrado\n\
             ao gerenciador de arquivos.",
        )
        .build();
    status.add_css_class("compact");

    // Feature cards — cada um abre um picker de imagens e relança o
    // próprio binário no modo certo (viewer, --dialog=batch, etc.).
    // Vai-e-volta evita reimplementar o pipeline em cima do dashboard.
    let features =
        adw::PreferencesGroup::builder().title("O que dá pra fazer").margin_top(16).build();
    for (title, subtitle, icon, action) in FEATURE_HIGHLIGHTS {
        let row = adw::ActionRow::builder()
            .title(*title)
            .subtitle(*subtitle)
            .activatable(action.is_some())
            .build();
        let icon_widget = gtk::Image::from_icon_name(icon);
        icon_widget.add_css_class("dim-label");
        row.add_prefix(&icon_widget);
        if let Some(cmd) = action {
            let arrow = gtk::Image::from_icon_name("go-next-symbolic");
            arrow.add_css_class("dim-label");
            row.add_suffix(&arrow);
            let window = window.clone();
            let cmd = *cmd;
            row.connect_activated(move |_| dispatch_feature(&window, cmd));
        }
        features.add(&row);
    }

    // Bloco de ações: botão primário "Abrir imagens" + dica sobre o
    // clique-direito (principal caminho de uso).
    let actions = gtk::Box::new(gtk::Orientation::Vertical, 12);
    actions.set_halign(gtk::Align::Center);
    actions.set_margin_top(20);

    let open_btn = gtk::Button::builder()
        .label("Abrir imagens…")
        .css_classes(["pill", "suggested-action"])
        .build();
    {
        let window = window.clone();
        open_btn.connect_clicked(move |_| open_files_dialog(&window));
    }
    actions.append(&open_btn);

    let tip = gtk::Label::builder()
        .label(
            "💡 Clique com o direito em imagens no Nautilus / Dolphin / \
             Nemo / Thunar para ver todas as operações rápidas.",
        )
        .justify(gtk::Justification::Center)
        .wrap(true)
        .max_width_chars(52)
        .css_classes(["dim-label"])
        .build();
    actions.append(&tip);

    // Empilha features + ações no slot de child do StatusPage
    let child = gtk::Box::new(gtk::Orientation::Vertical, 0);
    child.set_halign(gtk::Align::Center);
    child.append(&features);
    child.append(&actions);
    status.set_child(Some(&child));

    status.upcast()
}

/// Ações dos cards do dashboard. `None` = card puramente informativo
/// (ex.: "Clique direito no gerenciador" — não tem como disparar do
/// dashboard). Caso contrário é a string passada a [`dispatch_feature`].
const FEATURE_HIGHLIGHTS: &[(&str, &str, &str, Option<&str>)] = &[
    (
        "Visualizar imagens",
        "JPEG, PNG, WebP, AVIF, GIF, TIFF, BMP e RAW de câmeras",
        "image-x-generic-symbolic",
        Some("viewer"),
    ),
    (
        "Converter em lote",
        "Formato, qualidade e presets prontos para WhatsApp, Instagram, Facebook e Discord",
        "document-save-as-symbolic",
        Some("batch"),
    ),
    (
        "Ajustar cores",
        "Brilho, contraste, saturação e gamma — em lote",
        "image-adjust-symbolic",
        Some("adjust"),
    ),
    (
        "Remover fundo (IA)",
        "BiRefNet 100% local, sem enviar fotos para a nuvem",
        "applications-science-symbolic",
        Some("remove-bg"),
    ),
    (
        "Criar GIF animado",
        "A partir de uma sequência de frames, com controle de loop e fps",
        "video-x-generic-symbolic",
        Some("animate"),
    ),
    (
        "Clique direito no gerenciador",
        "Integração com Nautilus, Dolphin, Nemo, Thunar, PCManFM-Qt",
        "view-list-symbolic",
        None,
    ),
];

/// Pega os arquivos escolhidos e relança o próprio binário no modo
/// apropriado. Tratamos "viewer" como caso-especial (executável sem
/// subcomando); o resto vira `--dialog=<nome>`, exceto `remove-bg` que
/// é um subcomando CLI dedicado.
fn dispatch_feature(window: &adw::ApplicationWindow, action: &'static str) {
    let dialog = gtk::FileDialog::builder()
        .title(match action {
            "viewer" => "Selecionar imagens para visualizar",
            "batch" => "Selecionar imagens para conversão em lote",
            "adjust" => "Selecionar imagens para ajustar cores",
            "remove-bg" => "Selecionar imagens para remover fundo",
            "animate" => "Selecionar frames na ordem do GIF",
            _ => "Selecionar imagens",
        })
        .modal(true)
        .build();
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("Imagens"));
    for mime in [
        "image/jpeg",
        "image/png",
        "image/webp",
        "image/avif",
        "image/gif",
        "image/tiff",
        "image/bmp",
        "image/heic",
        "image/heif",
        "image/jxl",
    ] {
        filter.add_mime_type(mime);
    }
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    dialog.set_filters(Some(&filters));

    let parent = window.clone();
    let window = window.clone();
    dialog.open_multiple(Some(&parent), None::<&gio::Cancellable>, move |result| {
        let Ok(files) = result else { return };
        let paths: Vec<PathBuf> = (0..files.n_items())
            .filter_map(|i| files.item(i))
            .filter_map(|o| o.downcast::<gio::File>().ok())
            .filter_map(|f| f.path())
            .collect();
        if paths.is_empty() {
            return;
        }

        let Ok(exe) = std::env::current_exe() else { return };
        let mut cmd = std::process::Command::new(exe);
        match action {
            "viewer" => {} // sem args: abre direto no visualizador
            "remove-bg" => {
                cmd.arg("remove-bg");
            }
            other => {
                cmd.arg(format!("--dialog={other}"));
            }
        }
        cmd.args(paths.iter().map(|p| p.as_os_str()));
        let _ = cmd.spawn();
        window.close();
    });
}

/// File-chooser do dashboard: carrega uma lista de imagens e respawna o
/// processo com esses arquivos para reusar a lógica de carregamento do
/// CLI. Simples e previsível — evita duplicar o pipeline de load in-place.
fn open_files_dialog(window: &adw::ApplicationWindow) {
    let dialog = gtk::FileDialog::builder().title("Selecionar imagens").modal(true).build();
    // Filtro de imagens: tudo que o nosso decoder aguenta.
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("Imagens"));
    for mime in [
        "image/jpeg",
        "image/png",
        "image/webp",
        "image/avif",
        "image/gif",
        "image/tiff",
        "image/bmp",
        "image/heic",
        "image/heif",
        "image/jxl",
    ] {
        filter.add_mime_type(mime);
    }
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    dialog.set_filters(Some(&filters));

    let window = window.clone();
    let parent = window.clone();
    dialog.open_multiple(Some(&parent), None::<&gio::Cancellable>, move |result| {
        let Ok(files) = result else { return };
        let paths: Vec<PathBuf> = (0..files.n_items())
            .filter_map(|i| files.item(i))
            .filter_map(|o| o.downcast::<gio::File>().ok())
            .filter_map(|f| f.path())
            .collect();
        if paths.is_empty() {
            return;
        }
        // Reaproveita o binário atual: relança com os caminhos, o que
        // dispara o fluxo normal de viewer com lista de arquivos.
        let args: Vec<std::ffi::OsString> =
            paths.iter().map(|p| p.as_os_str().to_os_string()).collect();
        if let Ok(exe) = std::env::current_exe() {
            let _ = std::process::Command::new(exe).args(&args).spawn();
            window.close();
        }
    });
}

fn refresh_title(state: &Rc<RefCell<ViewerState>>) {
    let s = state.borrow();
    if s.files.is_empty() {
        return;
    }
    let path = &s.files[s.idx];
    let base_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    // `•` à esquerda sinaliza edições pendentes. Convenção Loupe/TextEditor.
    let name = if s.pending_ops.is_empty() { base_name } else { format!("• {base_name}") };
    let zoom_desc = match s.zoom {
        ZoomState::Fit => "Ajustar".to_string(),
        ZoomState::Scale(x) => format!("{}%", (x * 100.0).round() as i32),
    };
    let subtitle = format!("{} de {} · {}", s.idx + 1, s.files.len(), zoom_desc);
    s.title.set_title(&name);
    s.title.set_subtitle(&subtitle);
    drop(s);
    refresh_osd(state);
    refresh_film_strip_active(state);
}

type Histogram = [u32; 768];
type LoadedTexture = (gdk::Texture, (u32, u32), Histogram);

fn load_texture(path: &Path) -> Result<LoadedTexture, Box<dyn std::error::Error>> {
    let img = image::open(path)?;
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    let raw = rgba.into_raw();
    let histogram = compute_histogram(&raw);
    let stride = (w as usize) * 4;
    let bytes = glib::Bytes::from_owned(raw);
    let tex =
        gdk::MemoryTexture::new(w as i32, h as i32, gdk::MemoryFormat::R8g8b8a8, &bytes, stride);
    Ok((tex.upcast(), (w, h), histogram))
}

/// Conta ocorrências de cada byte (0..256) por canal em um buffer RGBA
/// contíguo. Ignora alpha — pro histograma do viewer o que importa é a
/// distribuição de luminância por canal. Retorna `[R..256, G..256, B..256]`
/// concatenado pra o rendering ficar com `.chunks(256)`.
fn compute_histogram(rgba: &[u8]) -> [u32; 768] {
    let mut h = [0u32; 768];
    for px in rgba.chunks_exact(4) {
        h[px[0] as usize] += 1;
        h[256 + px[1] as usize] += 1;
        h[512 + px[2] as usize] += 1;
    }
    h
}

// --------------------------------------------------------------------------
// Dialog modals (Prisma) — launched by service menus via `bigiris --dialog=X`.
// --------------------------------------------------------------------------

use bigimage_core::{
    adjust_file, convert_file, convert_file_to, crop_file, flip_file, make_gif, metadata,
    resize_file, rotate_file, AdjustOps, AnimateOptions, ConvertOutcome, CropRect, EncodeOptions,
    Filter, FlipAxis, Format, LoopMode, OverwritePolicy, PreviewOp, PreviewSession, ResizeMode,
    Rotation,
};
use image::DynamicImage;
use std::time::Duration;

/// Turn a `DynamicImage` into a `gdk::Texture` suitable for `gtk::Picture`.
/// Consumes the image (the RGBA buffer is handed to a `glib::Bytes` that
/// owns the allocation from there on).
fn dynamic_image_to_texture(img: DynamicImage) -> gdk::Texture {
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    let stride = (w as usize) * 4;
    let bytes = glib::Bytes::from_owned(rgba.into_raw());
    gdk::MemoryTexture::new(w as i32, h as i32, gdk::MemoryFormat::R8g8b8a8, &bytes, stride)
        .upcast()
}

/// Build the preview-area column every dialog uses on its right-hand side.
/// Returns the `gtk::Picture` the caller drives via `set_paintable` and the
/// outer container to append into the dialog's horizontal box.
fn build_preview_column(session: &PreviewSession) -> (gtk::Picture, gtk::Box) {
    let picture = gtk::Picture::new();
    picture.set_content_fit(gtk::ContentFit::Contain);
    picture.set_hexpand(true);
    picture.set_vexpand(true);
    picture.set_size_request(360, 360);

    // Initial paintable = unmodified thumbnail.
    let initial = dynamic_image_to_texture(session.thumbnail().clone());
    picture.set_paintable(Some(&initial));

    let heading =
        gtk::Label::builder().label("<b>Pré-visualização</b>").use_markup(true).xalign(0.0).build();

    let (nw, nh) = session.natural_size();
    let info = gtk::Label::builder().label(format!("Original: {nw}×{nh} px")).xalign(0.0).build();
    info.add_css_class("dim-label");

    let frame = gtk::Frame::new(None);
    frame.set_child(Some(&picture));

    let column = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .hexpand(true)
        .vexpand(true)
        .build();
    column.append(&heading);
    column.append(&frame);
    column.append(&info);

    (picture, column)
}

/// Wrap the GTK idle+timeout dance into a single handle. Callers schedule
/// a preview repaint by calling [`PreviewDebounce::trigger`], and the
/// closure runs at most once per debounce window no matter how many times
/// the sliders fire in the meantime.
struct PreviewDebounce {
    pending: std::cell::Cell<Option<glib::SourceId>>,
}

impl PreviewDebounce {
    fn new() -> Rc<Self> {
        Rc::new(Self { pending: std::cell::Cell::new(None) })
    }

    fn trigger<F: FnMut() + 'static>(self: &Rc<Self>, delay: Duration, mut f: F) {
        if let Some(id) = self.pending.take() {
            id.remove();
        }
        let weak = Rc::downgrade(self);
        let id = glib::timeout_add_local(delay, move || {
            f();
            if let Some(strong) = weak.upgrade() {
                strong.pending.set(None);
            }
            glib::ControlFlow::Break
        });
        self.pending.set(Some(id));
    }
}

/// Apply `op` to `session` and push the resulting thumbnail into `picture`.
/// Errors are logged and the previous paintable is left untouched.
fn refresh_preview(session: &PreviewSession, picture: &gtk::Picture, op: &PreviewOp) {
    match session.apply(op) {
        Ok(img) => picture.set_paintable(Some(&dynamic_image_to_texture(img))),
        Err(e) => tracing::warn!(error = %e, "preview: apply failed"),
    }
}

/// Seven target formats presented in the convert dialog. Order drives the
/// default: PNG first.
const CONVERT_FORMATS: &[(&str, Format)] = &[
    ("PNG", Format::Png),
    ("JPG (JPEG)", Format::Jpeg),
    ("WebP", Format::WebP),
    ("AVIF", Format::Avif),
    ("TIFF", Format::Tiff),
    ("BMP", Format::Bmp),
    ("GIF", Format::Gif),
];

/// Overwrite choices shared by convert and resize dialogs.
const OVERWRITE_CHOICES: &[(&str, OverwritePolicy)] = &[
    ("Pular se já existir", OverwritePolicy::Skip),
    ("Sobrescrever o arquivo", OverwritePolicy::Replace),
    ("Incrementar (_1, _2, …)", OverwritePolicy::Increment),
];

/// Interpolation filters for the resize dialog.
const RESIZE_FILTERS: &[(&str, Filter)] = &[
    ("Lanczos3 (padrão)", Filter::Lanczos3),
    ("Mitchell", Filter::Mitchell),
    ("CatmullRom", Filter::CatmullRom),
    ("Bilinear", Filter::Bilinear),
    ("Nearest (sem anti-aliasing)", Filter::Nearest),
];

/// Run the "Converter" modal. Returns GTK's exit code.
pub fn run_convert_dialog(files: Vec<PathBuf>) -> i32 {
    let app = adw::Application::builder().application_id(APP_ID).build();
    let files = Rc::new(files);
    app.connect_activate(move |app| {
        build_convert_dialog(app, files.clone()).present();
    });
    let code = app.run_with_args::<&str>(&[]);
    i32::from(u8::from(code))
}

fn build_convert_dialog(app: &adw::Application, files: Rc<Vec<PathBuf>>) -> adw::ApplicationWindow {
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .default_width(880)
        .default_height(500)
        .title("Converter imagens — Prisma")
        .build();

    let header = adw::HeaderBar::new();
    let title = adw::WindowTitle::new("Converter", "Prisma");
    header.set_title_widget(Some(&title));

    let preview_session = files.first().and_then(|p| PreviewSession::open(p).ok()).map(Rc::new);

    let format_names: Vec<&str> = CONVERT_FORMATS.iter().map(|(n, _)| *n).collect();
    let format_row = adw::ComboRow::builder()
        .title("Formato de destino")
        .model(&gtk::StringList::new(&format_names))
        .build();

    let overwrite_names: Vec<&str> = OVERWRITE_CHOICES.iter().map(|(n, _)| *n).collect();
    let overwrite_row = adw::ComboRow::builder()
        .title("Se o arquivo de saída existir")
        .model(&gtk::StringList::new(&overwrite_names))
        .build();

    let files_row = adw::ActionRow::builder()
        .title(format!("{} arquivo(s) selecionado(s)", files.len()))
        .subtitle(files_subtitle(&files))
        .build();

    let quality_spin = adw::SpinRow::with_range(1.0, 100.0, 1.0);
    quality_spin.set_title("Qualidade (JPEG/lossy)");
    quality_spin.set_subtitle("1 pior · 85 bom · 100 sem perdas");
    quality_spin.set_value(85.0);

    let progressive_switch = adw::SwitchRow::builder()
        .title("JPEG progressivo")
        .subtitle("carregamento em passes (formatos que suportam)")
        .active(false)
        .build();

    let optimize_switch = adw::SwitchRow::builder()
        .title("Otimizar tamanho")
        .subtitle("PNG com compressão máxima; mais lento, arquivo menor")
        .active(false)
        .build();

    let group = adw::PreferencesGroup::new();
    group.add(&files_row);
    group.add(&format_row);
    group.add(&quality_spin);
    group.add(&progressive_switch);
    group.add(&optimize_switch);
    group.add(&overwrite_row);

    let status = gtk::Label::builder().label("").wrap(true).xalign(0.0).build();
    status.add_css_class("dim-label");

    let cancel_btn = gtk::Button::with_label("Cancelar");
    let apply_btn = gtk::Button::builder().label("Converter").build();
    apply_btn.add_css_class("suggested-action");
    apply_btn.add_css_class("pill");

    let actions = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .halign(gtk::Align::End)
        .build();
    actions.append(&cancel_btn);
    actions.append(&apply_btn);

    // Smart prompt: if the source image has an alpha channel and the user
    // picks JPEG, warn that transparency will be flattened.
    let alpha_banner = adw::Banner::builder()
        .title("Esta imagem tem transparência — JPEG perde o canal alfa. Considere PNG ou WebP.")
        .revealed(false)
        .build();
    if let Some(sess) = preview_session.as_ref() {
        if sess.has_alpha_channel() {
            let format_row_clone = format_row.clone();
            let banner = alpha_banner.clone();
            let update = move || {
                let fmt = CONVERT_FORMATS[format_row_clone.selected() as usize].1;
                let warn = matches!(fmt, Format::Jpeg);
                banner.set_revealed(warn);
            };
            update();
            format_row.connect_selected_notify(move |_| update());
        }
    }

    // Smart prompt: LGPD / privacidade — se a imagem tem coordenadas GPS
    // no EXIF, avisa que o upload pode vazar a localização.
    let gps_banner = adw::Banner::builder()
        .title(
            "Esta imagem contém GPS (localização) no EXIF. Atenção ao compartilhar \
             publicamente — nosso pipeline remove o EXIF automaticamente no re-encode.",
        )
        .revealed(false)
        .build();
    gps_banner.add_css_class("warning");
    if let Some(sess) = preview_session.as_ref() {
        if sess.source_has_gps() {
            gps_banner.set_revealed(true);
        }
    }

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(16)
        .margin_top(16)
        .margin_bottom(16)
        .margin_start(16)
        .margin_end(16)
        .build();
    content.append(&gps_banner);
    content.append(&alpha_banner);
    content.append(&group);
    content.append(&status);
    content.append(&actions);
    content.set_size_request(420, -1);

    let main_box =
        gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(16).build();
    main_box.append(&content);
    if let Some(sess) = preview_session.as_ref() {
        let (_picture, column) = build_preview_column(sess);
        column.set_margin_top(16);
        column.set_margin_bottom(16);
        column.set_margin_end(16);
        main_box.append(&column);
        // Convert is format-level — visual preview is just the source
        // thumbnail (pixels don't change meaningfully between PNG/JPG/WebP
        // at default quality). No refresh wiring needed.
    }

    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&header);
    toolbar_view.set_content(Some(&main_box));
    window.set_content(Some(&toolbar_view));

    {
        let window = window.clone();
        cancel_btn.connect_clicked(move |_| window.close());
    }

    {
        let files = files.clone();
        let format_row = format_row.clone();
        let overwrite_row = overwrite_row.clone();
        let quality_spin = quality_spin.clone();
        let progressive_switch = progressive_switch.clone();
        let optimize_switch = optimize_switch.clone();
        let status = status.clone();
        let apply_btn = apply_btn.clone();
        let cancel_btn = cancel_btn.clone();
        let window = window.clone();
        apply_btn.clone().connect_clicked(move |_| {
            let fmt_idx = format_row.selected() as usize;
            let policy_idx = overwrite_row.selected() as usize;
            let fmt = CONVERT_FORMATS[fmt_idx].1;
            let policy = OVERWRITE_CHOICES[policy_idx].1;
            let opts = EncodeOptions {
                quality: Some(quality_spin.value() as u8),
                progressive: progressive_switch.is_active(),
                optimize: optimize_switch.is_active(),
            };

            apply_btn.set_sensitive(false);
            cancel_btn.set_sensitive(false);
            status.set_text("Convertendo…");

            let files = files.clone();
            let status = status.clone();
            let window = window.clone();
            let apply_btn = apply_btn.clone();
            let cancel_btn = cancel_btn.clone();
            glib::idle_add_local_once(move || {
                let (ok, skip, fail, first_err) = run_convert_batch(&files, fmt, &opts, policy);
                finish_dialog(&status, &apply_btn, &cancel_btn, &window, ok, skip, fail, first_err);
            });
        });
    }

    window
}

/// Run the "Redimensionar" modal.
pub fn run_resize_dialog(files: Vec<PathBuf>) -> i32 {
    let app = adw::Application::builder().application_id(APP_ID).build();
    let files = Rc::new(files);
    app.connect_activate(move |app| {
        build_resize_dialog(app, files.clone()).present();
    });
    let code = app.run_with_args::<&str>(&[]);
    i32::from(u8::from(code))
}

fn build_resize_dialog(app: &adw::Application, files: Rc<Vec<PathBuf>>) -> adw::ApplicationWindow {
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .default_width(900)
        .default_height(640)
        .title("Redimensionar imagens — Prisma")
        .build();

    let header = adw::HeaderBar::new();
    let title = adw::WindowTitle::new("Redimensionar", "Prisma");
    header.set_title_widget(Some(&title));

    let preview_session = files.first().and_then(|p| PreviewSession::open(p).ok()).map(Rc::new);

    // ---- Mode selector ------------------------------------------------------

    let mode_names = ["Aresta máxima", "Escala (%)", "Dimensão exata (WxH)", "Encaixar em (WxH)"];
    let mode_row =
        adw::ComboRow::builder().title("Modo").model(&gtk::StringList::new(&mode_names)).build();

    // Per-mode parameter pages live inside a Stack; switching the combo swaps
    // the page. Each page owns its own SpinRows so we can read values back.
    let max_edge_spin = adw::SpinRow::with_range(1.0, 16384.0, 1.0);
    max_edge_spin.set_title("Aresta máxima");
    max_edge_spin.set_subtitle("pixels (lado maior)");
    max_edge_spin.set_value(1920.0);

    let percent_spin = adw::SpinRow::with_range(1.0, 1000.0, 1.0);
    percent_spin.set_title("Porcentagem");
    percent_spin.set_subtitle("100 = tamanho original");
    percent_spin.set_value(50.0);

    let exact_w = adw::SpinRow::with_range(1.0, 16384.0, 1.0);
    exact_w.set_title("Largura");
    exact_w.set_value(1920.0);
    let exact_h = adw::SpinRow::with_range(1.0, 16384.0, 1.0);
    exact_h.set_title("Altura");
    exact_h.set_value(1080.0);

    let fit_w = adw::SpinRow::with_range(1.0, 16384.0, 1.0);
    fit_w.set_title("Largura máxima");
    fit_w.set_value(1920.0);
    let fit_h = adw::SpinRow::with_range(1.0, 16384.0, 1.0);
    fit_h.set_title("Altura máxima");
    fit_h.set_value(1080.0);

    let page_max_edge = adw::PreferencesGroup::new();
    page_max_edge.add(&max_edge_spin);
    let page_percent = adw::PreferencesGroup::new();
    page_percent.add(&percent_spin);
    let page_exact = adw::PreferencesGroup::new();
    page_exact.add(&exact_w);
    page_exact.add(&exact_h);
    let page_fit = adw::PreferencesGroup::new();
    page_fit.add(&fit_w);
    page_fit.add(&fit_h);

    let stack = gtk::Stack::new();
    stack.add_named(&page_max_edge, Some("max-edge"));
    stack.add_named(&page_percent, Some("percent"));
    stack.add_named(&page_exact, Some("exact"));
    stack.add_named(&page_fit, Some("fit"));
    stack.set_visible_child_name("max-edge");

    {
        let stack = stack.clone();
        mode_row.connect_selected_notify(move |combo| {
            let name = match combo.selected() {
                0 => "max-edge",
                1 => "percent",
                2 => "exact",
                _ => "fit",
            };
            stack.set_visible_child_name(name);
        });
    }

    // ---- Filter + format + overwrite ---------------------------------------

    let filter_names: Vec<&str> = RESIZE_FILTERS.iter().map(|(n, _)| *n).collect();
    let filter_row = adw::ComboRow::builder()
        .title("Kernel de interpolação")
        .model(&gtk::StringList::new(&filter_names))
        .build();

    let mut target_names = vec!["Manter (mesmo formato da origem)"];
    target_names.extend(CONVERT_FORMATS.iter().map(|(n, _)| *n));
    let target_row = adw::ComboRow::builder()
        .title("Formato de destino")
        .model(&gtk::StringList::new(&target_names))
        .build();

    let overwrite_names: Vec<&str> = OVERWRITE_CHOICES.iter().map(|(n, _)| *n).collect();
    let overwrite_row = adw::ComboRow::builder()
        .title("Se o arquivo de saída existir")
        .model(&gtk::StringList::new(&overwrite_names))
        .build();

    let files_row = adw::ActionRow::builder()
        .title(format!("{} arquivo(s) selecionado(s)", files.len()))
        .subtitle(files_subtitle(&files))
        .build();

    let group_files = adw::PreferencesGroup::new();
    group_files.add(&files_row);

    let group_mode = adw::PreferencesGroup::builder().title("Modo de redimensionamento").build();
    group_mode.add(&mode_row);

    let group_options = adw::PreferencesGroup::builder().title("Opções").build();
    group_options.add(&filter_row);
    group_options.add(&target_row);
    group_options.add(&overwrite_row);

    let status = gtk::Label::builder().label("").wrap(true).xalign(0.0).build();
    status.add_css_class("dim-label");

    let cancel_btn = gtk::Button::with_label("Cancelar");
    let apply_btn = gtk::Button::builder().label("Redimensionar").build();
    apply_btn.add_css_class("suggested-action");
    apply_btn.add_css_class("pill");

    let actions = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .halign(gtk::Align::End)
        .build();
    actions.append(&cancel_btn);
    actions.append(&apply_btn);

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(16)
        .margin_top(16)
        .margin_bottom(16)
        .margin_start(16)
        .margin_end(16)
        .build();
    content.append(&group_files);
    content.append(&group_mode);
    content.append(&stack);
    content.append(&group_options);
    content.append(&status);
    content.append(&actions);

    // Scroll the whole content in case the resize dialog overflows a small
    // screen — it's taller than the convert dialog.
    let scrolled = gtk::ScrolledWindow::builder()
        .hexpand(true)
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .child(&content)
        .build();
    scrolled.set_size_request(420, -1);

    let main_box =
        gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(16).build();
    main_box.append(&scrolled);
    let preview_picture: Option<gtk::Picture> = if let Some(sess) = preview_session.as_ref() {
        let (picture, column) = build_preview_column(sess);
        column.set_margin_top(16);
        column.set_margin_bottom(16);
        column.set_margin_end(16);
        main_box.append(&column);
        Some(picture)
    } else {
        None
    };

    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&header);
    toolbar_view.set_content(Some(&main_box));
    window.set_content(Some(&toolbar_view));

    // Live preview: any change in mode / filter / dimensions re-renders.
    if let (Some(picture), Some(session)) = (preview_picture.as_ref(), preview_session.as_ref()) {
        let debounce = PreviewDebounce::new();
        let mode_row_clone = mode_row.clone();
        let filter_row_clone = filter_row.clone();
        let max_edge_clone = max_edge_spin.clone();
        let percent_clone = percent_spin.clone();
        let exact_w_clone = exact_w.clone();
        let exact_h_clone = exact_h.clone();
        let fit_w_clone = fit_w.clone();
        let fit_h_clone = fit_h.clone();
        let schedule = {
            let debounce = debounce.clone();
            let session = session.clone();
            let picture = picture.clone();
            let mode_row = mode_row_clone.clone();
            let filter_row = filter_row_clone.clone();
            let max_edge_spin = max_edge_clone.clone();
            let percent_spin = percent_clone.clone();
            let exact_w = exact_w_clone.clone();
            let exact_h = exact_h_clone.clone();
            let fit_w = fit_w_clone.clone();
            let fit_h = fit_h_clone.clone();
            Rc::new(move || {
                let mode = match mode_row.selected() {
                    0 => ResizeMode::MaxEdge(max_edge_spin.value() as u32),
                    1 => ResizeMode::Percent(percent_spin.value() as f32),
                    2 => ResizeMode::Exact {
                        width: exact_w.value() as u32,
                        height: exact_h.value() as u32,
                    },
                    _ => ResizeMode::Fit {
                        width: fit_w.value() as u32,
                        height: fit_h.value() as u32,
                    },
                };
                let filter = RESIZE_FILTERS[filter_row.selected() as usize].1;
                let session = session.clone();
                let picture = picture.clone();
                debounce.trigger(Duration::from_millis(120), move || {
                    refresh_preview(&session, &picture, &PreviewOp::Resize { mode, filter });
                });
            })
        };

        // Mode / filter combo changes
        {
            let s = schedule.clone();
            mode_row_clone.connect_selected_notify(move |_| s());
        }
        {
            let s = schedule.clone();
            filter_row_clone.connect_selected_notify(move |_| s());
        }
        // Spin changes in each mode page
        for row in
            &[max_edge_clone, percent_clone, exact_w_clone, exact_h_clone, fit_w_clone, fit_h_clone]
        {
            let s = schedule.clone();
            row.connect_changed(move |_| s());
        }
    }

    {
        let window = window.clone();
        cancel_btn.connect_clicked(move |_| window.close());
    }

    {
        let files = files.clone();
        let mode_row = mode_row.clone();
        let filter_row = filter_row.clone();
        let target_row = target_row.clone();
        let overwrite_row = overwrite_row.clone();
        let max_edge_spin = max_edge_spin.clone();
        let percent_spin = percent_spin.clone();
        let exact_w = exact_w.clone();
        let exact_h = exact_h.clone();
        let fit_w = fit_w.clone();
        let fit_h = fit_h.clone();
        let status = status.clone();
        let apply_btn = apply_btn.clone();
        let cancel_btn = cancel_btn.clone();
        let window = window.clone();
        apply_btn.clone().connect_clicked(move |_| {
            let mode = match mode_row.selected() {
                0 => ResizeMode::MaxEdge(max_edge_spin.value() as u32),
                1 => ResizeMode::Percent(percent_spin.value() as f32),
                2 => ResizeMode::Exact {
                    width: exact_w.value() as u32,
                    height: exact_h.value() as u32,
                },
                _ => ResizeMode::Fit { width: fit_w.value() as u32, height: fit_h.value() as u32 },
            };
            let filter = RESIZE_FILTERS[filter_row.selected() as usize].1;
            let target: Option<Format> = match target_row.selected() {
                0 => None,
                n => Some(CONVERT_FORMATS[(n as usize) - 1].1),
            };
            let policy = OVERWRITE_CHOICES[overwrite_row.selected() as usize].1;
            // Resize dialog doesn't expose quality today — uses defaults.
            // Users who need quality control go through Convert ▸ Personalizar…
            // or chain via CLI.
            let opts = EncodeOptions::default();

            apply_btn.set_sensitive(false);
            cancel_btn.set_sensitive(false);
            status.set_text("Redimensionando…");

            let files = files.clone();
            let status = status.clone();
            let window = window.clone();
            let apply_btn = apply_btn.clone();
            let cancel_btn = cancel_btn.clone();
            glib::idle_add_local_once(move || {
                let (ok, skip, fail, first_err) =
                    run_resize_batch(&files, mode, filter, target, &opts, policy);
                finish_dialog(&status, &apply_btn, &cancel_btn, &window, ok, skip, fail, first_err);
            });
        });
    }

    window
}

fn files_subtitle(files: &[PathBuf]) -> String {
    if files.is_empty() {
        return "—".to_string();
    }
    let first = files[0].file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if files.len() == 1 {
        first
    } else {
        format!("{first} +{} outros", files.len() - 1)
    }
}

fn run_convert_batch(
    files: &[PathBuf],
    target: Format,
    opts: &EncodeOptions,
    policy: OverwritePolicy,
) -> (usize, usize, usize, Option<String>) {
    let mut ok = 0usize;
    let mut skip = 0usize;
    let mut fail = 0usize;
    let mut first_err: Option<String> = None;
    for f in files {
        match convert_file(f, target, opts, policy) {
            Ok(ConvertOutcome::Written { .. }) => ok += 1,
            Ok(ConvertOutcome::Skipped { .. }) => skip += 1,
            Err(e) => {
                fail += 1;
                if first_err.is_none() {
                    first_err = Some(format!("{}: {e}", f.display()));
                }
            }
        }
    }
    (ok, skip, fail, first_err)
}

fn run_resize_batch(
    files: &[PathBuf],
    mode: ResizeMode,
    filter: Filter,
    target: Option<Format>,
    opts: &EncodeOptions,
    policy: OverwritePolicy,
) -> (usize, usize, usize, Option<String>) {
    let mut ok = 0usize;
    let mut skip = 0usize;
    let mut fail = 0usize;
    let mut first_err: Option<String> = None;
    for f in files {
        match resize_file(f, mode, filter, target, opts, policy) {
            Ok(ConvertOutcome::Written { .. }) => ok += 1,
            Ok(ConvertOutcome::Skipped { .. }) => skip += 1,
            Err(e) => {
                fail += 1;
                if first_err.is_none() {
                    first_err = Some(format!("{}: {e}", f.display()));
                }
            }
        }
    }
    (ok, skip, fail, first_err)
}

#[allow(clippy::too_many_arguments)]
fn finish_dialog(
    status: &gtk::Label,
    apply_btn: &gtk::Button,
    cancel_btn: &gtk::Button,
    window: &adw::ApplicationWindow,
    ok: usize,
    skip: usize,
    fail: usize,
    first_err: Option<String>,
) {
    let msg = format!("{ok} gravado(s), {skip} ignorado(s), {fail} falha(s)");
    status.set_text(&msg);
    if let Some(err) = first_err {
        status.set_text(&format!("{msg}\n{err}"));
    }
    apply_btn.set_sensitive(true);
    cancel_btn.set_sensitive(true);
    if fail == 0 {
        // Auto-close on clean success so service-menu callers don't have to
        // click Cancel.
        let window = window.clone();
        glib::timeout_add_seconds_local_once(2, move || window.close());
    }
}

// ---------------------------------------------------------------------------
// Rotate / Flip / Adjust dialogs — built on top of the same run_*_batch +
// finish_dialog primitives used by convert/resize, just with different forms.
// ---------------------------------------------------------------------------

/// Rotate modal: pick 90 / 180 / 270 and an optional target format.
pub fn run_rotate_dialog(files: Vec<PathBuf>) -> i32 {
    let app = adw::Application::builder().application_id(APP_ID).build();
    let files = Rc::new(files);
    app.connect_activate(move |app| {
        build_rotate_dialog(app, files.clone()).present();
    });
    let code = app.run_with_args::<&str>(&[]);
    i32::from(u8::from(code))
}

fn build_rotate_dialog(app: &adw::Application, files: Rc<Vec<PathBuf>>) -> adw::ApplicationWindow {
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .default_width(440)
        .default_height(340)
        .title("Girar imagens — Prisma")
        .build();

    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&adw::WindowTitle::new("Girar", "Prisma")));

    let rotations = ["90° (horário)", "180°", "270° (anti-horário)"];
    let rotation_row =
        adw::ComboRow::builder().title("Rotação").model(&gtk::StringList::new(&rotations)).build();

    let mut target_names = vec!["Manter (mesmo formato da origem)"];
    target_names.extend(CONVERT_FORMATS.iter().map(|(n, _)| *n));
    let target_row = adw::ComboRow::builder()
        .title("Formato de destino")
        .model(&gtk::StringList::new(&target_names))
        .build();

    let overwrite_row = adw::ComboRow::builder()
        .title("Se o arquivo de saída existir")
        .model(&gtk::StringList::new(
            &OVERWRITE_CHOICES.iter().map(|(n, _)| *n).collect::<Vec<_>>(),
        ))
        .build();

    let files_row = adw::ActionRow::builder()
        .title(format!("{} arquivo(s) selecionado(s)", files.len()))
        .subtitle(files_subtitle(&files))
        .build();

    let group = adw::PreferencesGroup::new();
    group.add(&files_row);
    group.add(&rotation_row);
    group.add(&target_row);
    group.add(&overwrite_row);

    let status = gtk::Label::builder().label("").wrap(true).xalign(0.0).build();
    status.add_css_class("dim-label");

    let cancel_btn = gtk::Button::with_label("Cancelar");
    let apply_btn = gtk::Button::builder().label("Girar").build();
    apply_btn.add_css_class("suggested-action");
    apply_btn.add_css_class("pill");

    let actions = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .halign(gtk::Align::End)
        .build();
    actions.append(&cancel_btn);
    actions.append(&apply_btn);

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(16)
        .margin_top(16)
        .margin_bottom(16)
        .margin_start(16)
        .margin_end(16)
        .build();
    content.append(&group);
    content.append(&status);
    content.append(&actions);

    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&header);
    toolbar_view.set_content(Some(&content));
    window.set_content(Some(&toolbar_view));

    {
        let window = window.clone();
        cancel_btn.connect_clicked(move |_| window.close());
    }

    {
        let files = files.clone();
        let rotation_row = rotation_row.clone();
        let target_row = target_row.clone();
        let overwrite_row = overwrite_row.clone();
        let status = status.clone();
        let apply_btn = apply_btn.clone();
        let cancel_btn = cancel_btn.clone();
        let window = window.clone();
        apply_btn.clone().connect_clicked(move |_| {
            let rotation = match rotation_row.selected() {
                0 => Rotation::Deg90,
                1 => Rotation::Deg180,
                _ => Rotation::Deg270,
            };
            let target: Option<Format> = match target_row.selected() {
                0 => None,
                n => Some(CONVERT_FORMATS[(n as usize) - 1].1),
            };
            let policy = OVERWRITE_CHOICES[overwrite_row.selected() as usize].1;

            apply_btn.set_sensitive(false);
            cancel_btn.set_sensitive(false);
            status.set_text("Girando…");

            let files = files.clone();
            let status = status.clone();
            let window = window.clone();
            let apply_btn = apply_btn.clone();
            let cancel_btn = cancel_btn.clone();
            glib::idle_add_local_once(move || {
                let (ok, skip, fail, first_err) =
                    run_rotate_batch(&files, rotation, target, policy);
                finish_dialog(&status, &apply_btn, &cancel_btn, &window, ok, skip, fail, first_err);
            });
        });
    }

    window
}

fn run_rotate_batch(
    files: &[PathBuf],
    rotation: Rotation,
    target: Option<Format>,
    policy: OverwritePolicy,
) -> (usize, usize, usize, Option<String>) {
    let mut ok = 0;
    let mut skip = 0;
    let mut fail = 0;
    let mut first_err = None;
    for f in files {
        match rotate_file(f, rotation, target, policy) {
            Ok(ConvertOutcome::Written { .. }) => ok += 1,
            Ok(ConvertOutcome::Skipped { .. }) => skip += 1,
            Err(e) => {
                fail += 1;
                if first_err.is_none() {
                    first_err = Some(format!("{}: {e}", f.display()));
                }
            }
        }
    }
    (ok, skip, fail, first_err)
}

/// Default crop: a centred 50% rectangle — a safe non-empty starting
/// point for the UI that's unlikely to match the full image.
fn default_crop(img_w: u32, img_h: u32) -> CropRect {
    let w = (img_w / 2).max(1);
    let h = (img_h / 2).max(1);
    let x = (img_w.saturating_sub(w)) / 2;
    let y = (img_h.saturating_sub(h)) / 2;
    CropRect::new(x, y, w, h)
}

/// What the user is doing while the GestureDrag is active on the crop
/// preview. Decided at `drag_begin` from the hit-test against the
/// current rect; stays constant until drag ends.
#[derive(Clone, Copy, Debug, PartialEq)]
enum CropDragMode {
    /// Click+drag outside the rect (or on an empty canvas) grows a new
    /// rect starting at the click point.
    NewSelection {
        /// Click position in natural-image pixels.
        anchor_nat: (f64, f64),
    },
    /// Click strictly inside the current rect → translate it; its size
    /// is preserved. `rect0` is the rect at drag start (x, y, w, h),
    /// `click_nat` the click point in natural pixels.
    Move { rect0: (f64, f64, f64, f64), click_nat: (f64, f64) },
    /// Click on a corner handle → resize anchored at the opposite
    /// corner. `anchor_nat` is the fixed corner, `click_nat` the moving
    /// one at drag start.
    ResizeCorner { anchor_nat: (f64, f64), click_nat: (f64, f64) },
}

/// Widget-pixel tolerance for corner-handle hit-testing.
const CROP_HANDLE_TOLERANCE_WIDGET_PX: f64 = 12.0;

/// Decide the drag mode from the click's position relative to the
/// current rect. `fit` is `(scale, ox, oy)` from [`contain_fit`];
/// `rect` is the current crop in natural-image pixels.
fn decide_drag_mode(
    click_widget: (f64, f64),
    rect: (f64, f64, f64, f64),
    fit: (f64, f64, f64),
    natural: (u32, u32),
) -> CropDragMode {
    let (sx, sy) = click_widget;
    let (rx, ry, rw, rh) = rect;
    let (scale, ox, oy) = fit;
    let nat_w = natural.0 as f64;
    let nat_h = natural.1 as f64;
    let click_nat = (((sx - ox) / scale).clamp(0.0, nat_w), ((sy - oy) / scale).clamp(0.0, nat_h));

    // Widget-space corners of the current rect.
    let corners_nat = [
        (rx, ry),           // 0: top-left      ↔ 3 bottom-right
        (rx + rw, ry),      // 1: top-right     ↔ 2 bottom-left
        (rx, ry + rh),      // 2: bottom-left   ↔ 1 top-right
        (rx + rw, ry + rh), // 3: bottom-right  ↔ 0 top-left
    ];
    let corners_widget: [(f64, f64); 4] = core::array::from_fn(|i| {
        let (nx, ny) = corners_nat[i];
        (ox + nx * scale, oy + ny * scale)
    });
    let tol2 = CROP_HANDLE_TOLERANCE_WIDGET_PX * CROP_HANDLE_TOLERANCE_WIDGET_PX;
    for (i, (cx, cy)) in corners_widget.iter().enumerate() {
        let d2 = (sx - cx).powi(2) + (sy - cy).powi(2);
        if d2 <= tol2 {
            let anchor_nat = corners_nat[3 - i];
            return CropDragMode::ResizeCorner { anchor_nat, click_nat: corners_nat[i] };
        }
    }

    // Inside the rect (strict) → move.
    let inside =
        click_nat.0 > rx && click_nat.0 < rx + rw && click_nat.1 > ry && click_nat.1 < ry + rh;
    if inside {
        return CropDragMode::Move { rect0: rect, click_nat };
    }

    CropDragMode::NewSelection { anchor_nat: click_nat }
}

/// Apply a drag delta (already in *natural-pixel* units via `(dx/scale,
/// dy/scale)`) to `mode` and return the resulting `(x, y, w, h)` in
/// natural pixels, clamped to image bounds. Width/height never collapse
/// below 1 so the rect stays visible.
fn apply_drag(
    mode: CropDragMode,
    delta_nat: (f64, f64),
    natural: (u32, u32),
) -> (f64, f64, f64, f64) {
    let nat_w = natural.0 as f64;
    let nat_h = natural.1 as f64;
    let (dx, dy) = delta_nat;
    match mode {
        CropDragMode::NewSelection { anchor_nat: (ax, ay) } => {
            let cx = (ax + dx).clamp(0.0, nat_w);
            let cy = (ay + dy).clamp(0.0, nat_h);
            let x0 = ax.min(cx);
            let y0 = ay.min(cy);
            let w = (ax - cx).abs().max(1.0);
            let h = (ay - cy).abs().max(1.0);
            (x0, y0, w, h)
        }
        CropDragMode::Move { rect0: (rx, ry, rw, rh), click_nat: _ } => {
            // Preserve size; translate within bounds.
            let max_x = (nat_w - rw).max(0.0);
            let max_y = (nat_h - rh).max(0.0);
            let new_x = (rx + dx).clamp(0.0, max_x);
            let new_y = (ry + dy).clamp(0.0, max_y);
            (new_x, new_y, rw, rh)
        }
        CropDragMode::ResizeCorner { anchor_nat: (ax, ay), click_nat: (cx0, cy0) } => {
            let cx = (cx0 + dx).clamp(0.0, nat_w);
            let cy = (cy0 + dy).clamp(0.0, nat_h);
            let x0 = ax.min(cx);
            let y0 = ay.min(cy);
            let w = (ax - cx).abs().max(1.0);
            let h = (ay - cy).abs().max(1.0);
            (x0, y0, w, h)
        }
    }
}

/// Layout math for a `gtk::Picture` with `ContentFit::Contain`: given
/// the widget's allocated `(ww, wh)` and the displayed image's natural
/// `(iw, ih)`, returns `(scale, ox, oy)` — the uniform scale factor and
/// top-left offset of the rendered image inside the widget box.
///
/// Returns a safe identity transform when any input is non-positive so the
/// caller can unconditionally multiply without a division guard.
fn contain_fit(ww: f64, wh: f64, iw: f64, ih: f64) -> (f64, f64, f64) {
    if ww <= 0.0 || wh <= 0.0 || iw <= 0.0 || ih <= 0.0 {
        return (1.0, 0.0, 0.0);
    }
    let s = (ww / iw).min(wh / ih);
    let disp_w = iw * s;
    let disp_h = ih * s;
    ((s), (ww - disp_w) / 2.0, (wh - disp_h) / 2.0)
}

/// Build the interactive preview column for the crop dialog: full
/// thumbnail in a `gtk::Picture`, plus a `gtk::DrawingArea` overlay that
/// strokes the selection rect. A `GestureDrag` on the overlay updates the
/// spin rows (which remain authoritative); spin changes queue a redraw.
fn build_crop_preview_column(
    session: &Rc<PreviewSession>,
    natural: (u32, u32),
    x_spin: &adw::SpinRow,
    y_spin: &adw::SpinRow,
    w_spin: &adw::SpinRow,
    h_spin: &adw::SpinRow,
) -> gtk::Box {
    let picture = gtk::Picture::new();
    picture.set_content_fit(gtk::ContentFit::Contain);
    picture.set_hexpand(true);
    picture.set_vexpand(true);
    picture.set_size_request(360, 360);
    picture.set_paintable(Some(&dynamic_image_to_texture(session.thumbnail().clone())));

    let selection_area = gtk::DrawingArea::builder().hexpand(true).vexpand(true).build();

    // Draw: two-pass — shade the outside of the selection rect, then
    // stroke the rect itself with a bright outline + corner handles.
    {
        let x_spin = x_spin.clone();
        let y_spin = y_spin.clone();
        let w_spin = w_spin.clone();
        let h_spin = h_spin.clone();
        selection_area.set_draw_func(move |_area, cr, ww, wh| {
            let ww = ww as f64;
            let wh = wh as f64;
            let (scale, ox, oy) = contain_fit(ww, wh, natural.0 as f64, natural.1 as f64);
            let rx = ox + x_spin.value() * scale;
            let ry = oy + y_spin.value() * scale;
            let rw = w_spin.value() * scale;
            let rh = h_spin.value() * scale;

            // Shade outside the rect (four bands) for clear selection focus.
            cr.set_source_rgba(0.0, 0.0, 0.0, 0.45);
            cr.rectangle(0.0, 0.0, ww, ry);
            cr.rectangle(0.0, ry + rh, ww, wh - (ry + rh));
            cr.rectangle(0.0, ry, rx, rh);
            cr.rectangle(rx + rw, ry, ww - (rx + rw), rh);
            let _ = cr.fill();

            // Selection outline.
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.95);
            cr.set_line_width(1.5);
            cr.rectangle(rx + 0.5, ry + 0.5, rw - 1.0, rh - 1.0);
            let _ = cr.stroke();

            // Corner handles — small squares to hint "drag me".
            cr.set_source_rgba(1.0, 1.0, 1.0, 1.0);
            let k = 5.0;
            for (cx, cy) in [(rx, ry), (rx + rw, ry), (rx, ry + rh), (rx + rw, ry + rh)] {
                cr.rectangle(cx - k / 2.0, cy - k / 2.0, k, k);
            }
            let _ = cr.fill();
        });
    }

    // Spin changes → queue redraw of the overlay.
    for spin in [x_spin, y_spin, w_spin, h_spin] {
        let area = selection_area.clone();
        spin.connect_changed(move |_| area.queue_draw());
    }

    // Drag semantics — three modes, decided at drag_begin from the click
    // point's position relative to the current rect:
    //   - on a corner handle  → ResizeCorner (opposite corner anchors)
    //   - strictly inside     → Move (rect translates, size preserved)
    //   - outside             → NewSelection (classic draw-a-box)
    let drag = gtk::GestureDrag::builder().button(1).build();
    let drag_mode: Rc<Cell<CropDragMode>> =
        Rc::new(Cell::new(CropDragMode::NewSelection { anchor_nat: (0.0, 0.0) }));
    {
        let area = selection_area.clone();
        let mode = drag_mode.clone();
        let x_spin = x_spin.clone();
        let y_spin = y_spin.clone();
        let w_spin = w_spin.clone();
        let h_spin = h_spin.clone();
        drag.connect_drag_begin(move |_, sx, sy| {
            let (scale, ox, oy) = contain_fit(
                area.width() as f64,
                area.height() as f64,
                natural.0 as f64,
                natural.1 as f64,
            );
            if scale <= 0.0 {
                return;
            }
            let rect = (x_spin.value(), y_spin.value(), w_spin.value(), h_spin.value());
            let decided = decide_drag_mode((sx, sy), rect, (scale, ox, oy), natural);
            mode.set(decided);
            if let CropDragMode::NewSelection { anchor_nat } = decided {
                // Seed a 1px rect anchored at the click so the overlay is
                // visible immediately — update handler will expand it.
                x_spin.set_value(anchor_nat.0);
                y_spin.set_value(anchor_nat.1);
                w_spin.set_value(1.0);
                h_spin.set_value(1.0);
            }
        });
    }
    {
        let area = selection_area.clone();
        let mode = drag_mode.clone();
        let x_spin = x_spin.clone();
        let y_spin = y_spin.clone();
        let w_spin = w_spin.clone();
        let h_spin = h_spin.clone();
        drag.connect_drag_update(move |_, dx, dy| {
            let (scale, _ox, _oy) = contain_fit(
                area.width() as f64,
                area.height() as f64,
                natural.0 as f64,
                natural.1 as f64,
            );
            if scale <= 0.0 {
                return;
            }
            let (x0, y0, w, h) = apply_drag(mode.get(), (dx / scale, dy / scale), natural);
            x_spin.set_value(x0);
            y_spin.set_value(y0);
            w_spin.set_value(w);
            h_spin.set_value(h);
        });
    }
    selection_area.add_controller(drag);

    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&picture));
    overlay.add_overlay(&selection_area);

    let column = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
    column.append(&overlay);

    let hint = gtk::Label::builder()
        .label(
            "Arraste nos cantos para redimensionar · dentro do recorte para mover · \
             fora dele para uma nova seleção.",
        )
        .wrap(true)
        .xalign(0.5)
        .margin_top(6)
        .build();
    hint.add_css_class("dim-label");
    column.append(&hint);

    column
}

/// Crop modal: rectangular window by (x, y, w, h) with live preview.
pub fn run_crop_dialog(files: Vec<PathBuf>) -> i32 {
    let app = adw::Application::builder().application_id(APP_ID).build();
    let files = Rc::new(files);
    app.connect_activate(move |app| {
        build_crop_dialog(app, files.clone()).present();
    });
    let code = app.run_with_args::<&str>(&[]);
    i32::from(u8::from(code))
}

fn build_crop_dialog(app: &adw::Application, files: Rc<Vec<PathBuf>>) -> adw::ApplicationWindow {
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .default_width(900)
        .default_height(560)
        .title("Recortar — Prisma")
        .build();

    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&adw::WindowTitle::new("Recortar", "Prisma")));

    let content_h = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(16)
        .margin_top(16)
        .margin_bottom(16)
        .margin_start(16)
        .margin_end(16)
        .build();

    // Preview is keyed off the first file — multi-file dialogs share the
    // rect and per-file validation catches out-of-bounds at apply time.
    let preview_session = files.first().and_then(|p| PreviewSession::open(p).ok()).map(Rc::new);
    let natural = preview_session.as_ref().map(|s| s.natural_size()).unwrap_or((1, 1));
    let initial = default_crop(natural.0, natural.1);

    // Form column — native-width controls for coordinates and size.
    let form_col = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .width_request(360)
        .build();

    let files_row = adw::ActionRow::builder()
        .title(format!("{} arquivo(s) selecionado(s)", files.len()))
        .subtitle(files_subtitle(&files))
        .build();

    let natural_row = adw::ActionRow::builder()
        .title("Tamanho original")
        .subtitle(format!("{} × {} px (primeiro arquivo)", natural.0, natural.1))
        .build();

    // Ranges clamp to the first file's natural dims. For multi-file sets
    // with different sizes, per-file validation handles the rest.
    let x_spin = adw::SpinRow::with_range(0.0, (natural.0.saturating_sub(1)) as f64, 1.0);
    x_spin.set_title("X (px)");
    x_spin.set_value(initial.x as f64);

    let y_spin = adw::SpinRow::with_range(0.0, (natural.1.saturating_sub(1)) as f64, 1.0);
    y_spin.set_title("Y (px)");
    y_spin.set_value(initial.y as f64);

    let w_spin = adw::SpinRow::with_range(1.0, natural.0 as f64, 1.0);
    w_spin.set_title("Largura (px)");
    w_spin.set_value(initial.width as f64);

    let h_spin = adw::SpinRow::with_range(1.0, natural.1 as f64, 1.0);
    h_spin.set_title("Altura (px)");
    h_spin.set_value(initial.height as f64);

    let mut target_names = vec!["Manter (mesmo formato da origem)"];
    target_names.extend(CONVERT_FORMATS.iter().map(|(n, _)| *n));
    let target_row = adw::ComboRow::builder()
        .title("Formato de destino")
        .model(&gtk::StringList::new(&target_names))
        .build();

    let overwrite_row = adw::ComboRow::builder()
        .title("Se o arquivo de saída existir")
        .model(&gtk::StringList::new(
            &OVERWRITE_CHOICES.iter().map(|(n, _)| *n).collect::<Vec<_>>(),
        ))
        .build();

    let group = adw::PreferencesGroup::new();
    group.add(&files_row);
    group.add(&natural_row);
    group.add(&x_spin);
    group.add(&y_spin);
    group.add(&w_spin);
    group.add(&h_spin);
    group.add(&target_row);
    group.add(&overwrite_row);
    form_col.append(&group);

    let status = gtk::Label::builder().label("").wrap(true).xalign(0.0).build();
    status.add_css_class("dim-label");
    form_col.append(&status);

    let cancel_btn = gtk::Button::with_label("Cancelar");
    let reset_btn = gtk::Button::with_label("Centro 50%");
    let apply_btn = gtk::Button::builder().label("Recortar").build();
    apply_btn.add_css_class("suggested-action");
    apply_btn.add_css_class("pill");

    let actions = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .halign(gtk::Align::End)
        .build();
    actions.append(&reset_btn);
    actions.append(&cancel_btn);
    actions.append(&apply_btn);
    form_col.append(&actions);

    content_h.append(&form_col);

    // Interactive preview: full thumbnail + DrawingArea overlay that strokes
    // the selection rect and shades the outside. Spin rows are the source of
    // truth; drag gestures update them and the overlay redraws.
    if let Some(sess) = preview_session.as_ref() {
        let column = build_crop_preview_column(sess, natural, &x_spin, &y_spin, &w_spin, &h_spin);
        content_h.append(&column);
    }

    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&header);
    toolbar_view.set_content(Some(&content_h));
    window.set_content(Some(&toolbar_view));

    // Reset button → re-centre to 50% of the first image.
    {
        let x_spin = x_spin.clone();
        let y_spin = y_spin.clone();
        let w_spin = w_spin.clone();
        let h_spin = h_spin.clone();
        reset_btn.connect_clicked(move |_| {
            let d = default_crop(natural.0, natural.1);
            w_spin.set_value(d.width as f64);
            h_spin.set_value(d.height as f64);
            x_spin.set_value(d.x as f64);
            y_spin.set_value(d.y as f64);
        });
    }

    {
        let window = window.clone();
        cancel_btn.connect_clicked(move |_| window.close());
    }

    {
        let files = files.clone();
        let x_spin = x_spin.clone();
        let y_spin = y_spin.clone();
        let w_spin = w_spin.clone();
        let h_spin = h_spin.clone();
        let target_row = target_row.clone();
        let overwrite_row = overwrite_row.clone();
        let status = status.clone();
        let apply_btn = apply_btn.clone();
        let cancel_btn = cancel_btn.clone();
        let window = window.clone();
        apply_btn.clone().connect_clicked(move |_| {
            let rect = CropRect::new(
                x_spin.value() as u32,
                y_spin.value() as u32,
                w_spin.value() as u32,
                h_spin.value() as u32,
            );
            let target: Option<Format> = match target_row.selected() {
                0 => None,
                n => Some(CONVERT_FORMATS[(n as usize) - 1].1),
            };
            let policy = OVERWRITE_CHOICES[overwrite_row.selected() as usize].1;

            apply_btn.set_sensitive(false);
            cancel_btn.set_sensitive(false);
            status.set_text("Recortando…");

            let files = files.clone();
            let status = status.clone();
            let window = window.clone();
            let apply_btn = apply_btn.clone();
            let cancel_btn = cancel_btn.clone();
            glib::idle_add_local_once(move || {
                let (ok, skip, fail, first_err) = run_crop_batch(&files, rect, target, policy);
                finish_dialog(&status, &apply_btn, &cancel_btn, &window, ok, skip, fail, first_err);
            });
        });
    }

    window
}

fn run_crop_batch(
    files: &[PathBuf],
    rect: CropRect,
    target: Option<Format>,
    policy: OverwritePolicy,
) -> (usize, usize, usize, Option<String>) {
    let mut ok = 0;
    let mut skip = 0;
    let mut fail = 0;
    let mut first_err = None;
    for f in files {
        match crop_file(f, rect, target, policy) {
            Ok(ConvertOutcome::Written { .. }) => ok += 1,
            Ok(ConvertOutcome::Skipped { .. }) => skip += 1,
            Err(e) => {
                fail += 1;
                if first_err.is_none() {
                    first_err = Some(format!("{}: {e}", f.display()));
                }
            }
        }
    }
    (ok, skip, fail, first_err)
}

/// Upscale factors we expose in the dialog — matches the CLI `--factor` range
/// (2..=4). Real-ESRGAN backend, when available, slots in without changing
/// this list.
const UPSCALE_FACTORS: &[(&str, u8)] = &[("2× (200%)", 2), ("3× (300%)", 3), ("4× (400%)", 4)];

/// Upscale modal: pick an integer factor and run Lanczos3 resize.
pub fn run_upscale_dialog(files: Vec<PathBuf>) -> i32 {
    let app = adw::Application::builder().application_id(APP_ID).build();
    let files = Rc::new(files);
    app.connect_activate(move |app| {
        build_upscale_dialog(app, files.clone()).present();
    });
    let code = app.run_with_args::<&str>(&[]);
    i32::from(u8::from(code))
}

fn build_upscale_dialog(app: &adw::Application, files: Rc<Vec<PathBuf>>) -> adw::ApplicationWindow {
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .default_width(820)
        .default_height(500)
        .title("Aumentar resolução — Prisma")
        .build();

    let header = adw::HeaderBar::new();
    header
        .set_title_widget(Some(&adw::WindowTitle::new("Aumentar resolução", "Prisma · Lanczos3")));

    let content_h = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(16)
        .margin_top(16)
        .margin_bottom(16)
        .margin_start(16)
        .margin_end(16)
        .build();

    let preview_session = files.first().and_then(|p| PreviewSession::open(p).ok()).map(Rc::new);
    let natural = preview_session.as_ref().map(|s| s.natural_size()).unwrap_or((0, 0));

    let form_col = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .width_request(340)
        .build();

    // Hook — shown when an IA backend lands. Today just an informative note.
    let engine_note = gtk::Label::builder()
        .label(
            "Motor: Lanczos3 (CPU). O modo IA (Real-ESRGAN) será habilitado \
             automaticamente quando um modelo ONNX FOSS estiver disponível.",
        )
        .wrap(true)
        .xalign(0.0)
        .build();
    engine_note.add_css_class("dim-label");

    let files_row = adw::ActionRow::builder()
        .title(format!("{} arquivo(s) selecionado(s)", files.len()))
        .subtitle(files_subtitle(&files))
        .build();

    let natural_row = adw::ActionRow::builder()
        .title("Tamanho original")
        .subtitle(if natural == (0, 0) {
            "—".to_string()
        } else {
            format!("{} × {} px (primeiro arquivo)", natural.0, natural.1)
        })
        .build();

    let factor_names: Vec<&str> = UPSCALE_FACTORS.iter().map(|(n, _)| *n).collect();
    let factor_row =
        adw::ComboRow::builder().title("Fator").model(&gtk::StringList::new(&factor_names)).build();

    let predicted_row = adw::ActionRow::builder().title("Destino previsto").subtitle("—").build();
    {
        let factor_row_c = factor_row.clone();
        let predicted_row_c = predicted_row.clone();
        let refresh_prediction = move || {
            let f = UPSCALE_FACTORS[factor_row_c.selected() as usize].1;
            let sub = if natural == (0, 0) {
                format!("× {f} (tamanho indisponível)")
            } else {
                let w = (natural.0 as u64) * (f as u64);
                let h = (natural.1 as u64) * (f as u64);
                format!("≈ {w} × {h} px")
            };
            predicted_row_c.set_subtitle(&sub);
        };
        refresh_prediction();
        factor_row.connect_selected_notify(move |_| refresh_prediction());
    }

    let mut target_names = vec!["Manter (mesmo formato da origem)"];
    target_names.extend(CONVERT_FORMATS.iter().map(|(n, _)| *n));
    let target_row = adw::ComboRow::builder()
        .title("Formato de destino")
        .model(&gtk::StringList::new(&target_names))
        .build();

    let overwrite_row = adw::ComboRow::builder()
        .title("Se o arquivo de saída existir")
        .model(&gtk::StringList::new(
            &OVERWRITE_CHOICES.iter().map(|(n, _)| *n).collect::<Vec<_>>(),
        ))
        .build();

    let group = adw::PreferencesGroup::new();
    group.add(&files_row);
    group.add(&natural_row);
    group.add(&factor_row);
    group.add(&predicted_row);
    group.add(&target_row);
    group.add(&overwrite_row);

    form_col.append(&engine_note);
    form_col.append(&group);

    let status = gtk::Label::builder().label("").wrap(true).xalign(0.0).build();
    status.add_css_class("dim-label");
    form_col.append(&status);

    let cancel_btn = gtk::Button::with_label("Cancelar");
    let apply_btn = gtk::Button::builder().label("Aumentar").build();
    apply_btn.add_css_class("suggested-action");
    apply_btn.add_css_class("pill");

    let actions = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .halign(gtk::Align::End)
        .build();
    actions.append(&cancel_btn);
    actions.append(&apply_btn);
    form_col.append(&actions);

    content_h.append(&form_col);

    // Preview column shows an upscaled thumbnail of the first file as a
    // sanity check — PreviewOp::Resize respects the Lanczos3 filter.
    if let Some(sess) = preview_session.as_ref() {
        let (picture, column) = build_preview_column(sess);
        content_h.append(&column);

        let refresh = {
            let session = sess.clone();
            let picture = picture.clone();
            let factor_row = factor_row.clone();
            move || {
                let f = UPSCALE_FACTORS[factor_row.selected() as usize].1;
                refresh_preview(
                    &session,
                    &picture,
                    &PreviewOp::Resize {
                        mode: ResizeMode::Percent(f32::from(f) * 100.0),
                        filter: Filter::Lanczos3,
                    },
                );
            }
        };
        refresh();
        factor_row.connect_selected_notify(move |_| refresh());
    }

    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&header);
    toolbar_view.set_content(Some(&content_h));
    window.set_content(Some(&toolbar_view));

    {
        let window = window.clone();
        cancel_btn.connect_clicked(move |_| window.close());
    }

    {
        let files = files.clone();
        let factor_row = factor_row.clone();
        let target_row = target_row.clone();
        let overwrite_row = overwrite_row.clone();
        let status = status.clone();
        let apply_btn = apply_btn.clone();
        let cancel_btn = cancel_btn.clone();
        let window = window.clone();
        apply_btn.clone().connect_clicked(move |_| {
            let factor = UPSCALE_FACTORS[factor_row.selected() as usize].1;
            let target: Option<Format> = match target_row.selected() {
                0 => None,
                n => Some(CONVERT_FORMATS[(n as usize) - 1].1),
            };
            let policy = OVERWRITE_CHOICES[overwrite_row.selected() as usize].1;

            apply_btn.set_sensitive(false);
            cancel_btn.set_sensitive(false);
            status.set_text("Aumentando…");

            let files = files.clone();
            let status = status.clone();
            let window = window.clone();
            let apply_btn = apply_btn.clone();
            let cancel_btn = cancel_btn.clone();
            glib::idle_add_local_once(move || {
                let (ok, skip, fail, first_err) = run_upscale_batch(&files, factor, target, policy);
                finish_dialog(&status, &apply_btn, &cancel_btn, &window, ok, skip, fail, first_err);
            });
        });
    }

    window
}

fn run_upscale_batch(
    files: &[PathBuf],
    factor: u8,
    target: Option<Format>,
    policy: OverwritePolicy,
) -> (usize, usize, usize, Option<String>) {
    let mode = ResizeMode::Percent(f32::from(factor) * 100.0);
    let opts = EncodeOptions::default();
    let mut ok = 0;
    let mut skip = 0;
    let mut fail = 0;
    let mut first_err = None;
    for f in files {
        match resize_file(f, mode, Filter::Lanczos3, target, &opts, policy) {
            Ok(ConvertOutcome::Written { .. }) => ok += 1,
            Ok(ConvertOutcome::Skipped { .. }) => skip += 1,
            Err(e) => {
                fail += 1;
                if first_err.is_none() {
                    first_err = Some(format!("{}: {e}", f.display()));
                }
            }
        }
    }
    (ok, skip, fail, first_err)
}

/// Flip modal: horizontal or vertical mirror.
pub fn run_flip_dialog(files: Vec<PathBuf>) -> i32 {
    let app = adw::Application::builder().application_id(APP_ID).build();
    let files = Rc::new(files);
    app.connect_activate(move |app| {
        build_flip_dialog(app, files.clone()).present();
    });
    let code = app.run_with_args::<&str>(&[]);
    i32::from(u8::from(code))
}

fn build_flip_dialog(app: &adw::Application, files: Rc<Vec<PathBuf>>) -> adw::ApplicationWindow {
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .default_width(440)
        .default_height(340)
        .title("Espelhar imagens — Prisma")
        .build();

    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&adw::WindowTitle::new("Espelhar", "Prisma")));

    let axes = ["Horizontal (esquerda ↔ direita)", "Vertical (cima ↔ baixo)"];
    let axis_row =
        adw::ComboRow::builder().title("Eixo").model(&gtk::StringList::new(&axes)).build();

    let mut target_names = vec!["Manter (mesmo formato da origem)"];
    target_names.extend(CONVERT_FORMATS.iter().map(|(n, _)| *n));
    let target_row = adw::ComboRow::builder()
        .title("Formato de destino")
        .model(&gtk::StringList::new(&target_names))
        .build();

    let overwrite_row = adw::ComboRow::builder()
        .title("Se o arquivo de saída existir")
        .model(&gtk::StringList::new(
            &OVERWRITE_CHOICES.iter().map(|(n, _)| *n).collect::<Vec<_>>(),
        ))
        .build();

    let files_row = adw::ActionRow::builder()
        .title(format!("{} arquivo(s) selecionado(s)", files.len()))
        .subtitle(files_subtitle(&files))
        .build();

    let group = adw::PreferencesGroup::new();
    group.add(&files_row);
    group.add(&axis_row);
    group.add(&target_row);
    group.add(&overwrite_row);

    let status = gtk::Label::builder().label("").wrap(true).xalign(0.0).build();
    status.add_css_class("dim-label");

    let cancel_btn = gtk::Button::with_label("Cancelar");
    let apply_btn = gtk::Button::builder().label("Espelhar").build();
    apply_btn.add_css_class("suggested-action");
    apply_btn.add_css_class("pill");

    let actions = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .halign(gtk::Align::End)
        .build();
    actions.append(&cancel_btn);
    actions.append(&apply_btn);

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(16)
        .margin_top(16)
        .margin_bottom(16)
        .margin_start(16)
        .margin_end(16)
        .build();
    content.append(&group);
    content.append(&status);
    content.append(&actions);

    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&header);
    toolbar_view.set_content(Some(&content));
    window.set_content(Some(&toolbar_view));

    {
        let window = window.clone();
        cancel_btn.connect_clicked(move |_| window.close());
    }

    {
        let files = files.clone();
        let axis_row = axis_row.clone();
        let target_row = target_row.clone();
        let overwrite_row = overwrite_row.clone();
        let status = status.clone();
        let apply_btn = apply_btn.clone();
        let cancel_btn = cancel_btn.clone();
        let window = window.clone();
        apply_btn.clone().connect_clicked(move |_| {
            let axis = match axis_row.selected() {
                0 => FlipAxis::Horizontal,
                _ => FlipAxis::Vertical,
            };
            let target: Option<Format> = match target_row.selected() {
                0 => None,
                n => Some(CONVERT_FORMATS[(n as usize) - 1].1),
            };
            let policy = OVERWRITE_CHOICES[overwrite_row.selected() as usize].1;

            apply_btn.set_sensitive(false);
            cancel_btn.set_sensitive(false);
            status.set_text("Espelhando…");

            let files = files.clone();
            let status = status.clone();
            let window = window.clone();
            let apply_btn = apply_btn.clone();
            let cancel_btn = cancel_btn.clone();
            glib::idle_add_local_once(move || {
                let (ok, skip, fail, first_err) = run_flip_batch(&files, axis, target, policy);
                finish_dialog(&status, &apply_btn, &cancel_btn, &window, ok, skip, fail, first_err);
            });
        });
    }

    window
}

fn run_flip_batch(
    files: &[PathBuf],
    axis: FlipAxis,
    target: Option<Format>,
    policy: OverwritePolicy,
) -> (usize, usize, usize, Option<String>) {
    let mut ok = 0;
    let mut skip = 0;
    let mut fail = 0;
    let mut first_err = None;
    for f in files {
        match flip_file(f, axis, target, policy) {
            Ok(ConvertOutcome::Written { .. }) => ok += 1,
            Ok(ConvertOutcome::Skipped { .. }) => skip += 1,
            Err(e) => {
                fail += 1;
                if first_err.is_none() {
                    first_err = Some(format!("{}: {e}", f.display()));
                }
            }
        }
    }
    (ok, skip, fail, first_err)
}

/// Adjust modal: brightness + contrast + saturation + gamma.
pub fn run_adjust_dialog(files: Vec<PathBuf>) -> i32 {
    let app = adw::Application::builder().application_id(APP_ID).build();
    let files = Rc::new(files);
    app.connect_activate(move |app| {
        build_adjust_dialog(app, files.clone()).present();
    });
    let code = app.run_with_args::<&str>(&[]);
    i32::from(u8::from(code))
}

fn build_adjust_dialog(app: &adw::Application, files: Rc<Vec<PathBuf>>) -> adw::ApplicationWindow {
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .default_width(880)
        .default_height(640)
        .title("Ajustar cores — Prisma")
        .build();

    // Preview session — loaded from the first file. Multi-select still works
    // (Aplicar opera em todos), mas o preview mostra o primeiro.
    let preview_session = files.first().and_then(|p| PreviewSession::open(p).ok()).map(Rc::new);

    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&adw::WindowTitle::new("Ajustar cores", "Prisma")));

    let brightness = adw::SpinRow::with_range(-100.0, 100.0, 1.0);
    brightness.set_title("Brilho");
    brightness.set_subtitle("-100 escuro · 0 neutro · +100 claro");
    brightness.set_value(0.0);

    let contrast = adw::SpinRow::with_range(-100.0, 100.0, 1.0);
    contrast.set_title("Contraste");
    contrast.set_subtitle("-100 plano · 0 neutro · +100 agressivo");
    contrast.set_value(0.0);

    let saturation = adw::SpinRow::with_range(-100.0, 100.0, 1.0);
    saturation.set_title("Saturação");
    saturation.set_subtitle("-100 cinza · 0 neutro · +100 vibrante");
    saturation.set_value(0.0);

    let gamma = adw::SpinRow::with_range(0.1, 10.0, 0.05);
    gamma.set_title("Gamma");
    gamma.set_subtitle("< 1 clareia midtones · 1 neutro · > 1 escurece");
    gamma.set_digits(2);
    gamma.set_value(1.0);

    let mut target_names = vec!["Manter (mesmo formato da origem)"];
    target_names.extend(CONVERT_FORMATS.iter().map(|(n, _)| *n));
    let target_row = adw::ComboRow::builder()
        .title("Formato de destino")
        .model(&gtk::StringList::new(&target_names))
        .build();

    let overwrite_row = adw::ComboRow::builder()
        .title("Se o arquivo de saída existir")
        .model(&gtk::StringList::new(
            &OVERWRITE_CHOICES.iter().map(|(n, _)| *n).collect::<Vec<_>>(),
        ))
        .build();

    let files_row = adw::ActionRow::builder()
        .title(format!("{} arquivo(s) selecionado(s)", files.len()))
        .subtitle(files_subtitle(&files))
        .build();

    let group_files = adw::PreferencesGroup::new();
    group_files.add(&files_row);

    let group_tone = adw::PreferencesGroup::builder().title("Tom").build();
    group_tone.add(&brightness);
    group_tone.add(&contrast);
    group_tone.add(&gamma);

    let group_color = adw::PreferencesGroup::builder().title("Cor").build();
    group_color.add(&saturation);

    let group_out = adw::PreferencesGroup::builder().title("Saída").build();
    group_out.add(&target_row);
    group_out.add(&overwrite_row);

    let status = gtk::Label::builder().label("").wrap(true).xalign(0.0).build();
    status.add_css_class("dim-label");

    let cancel_btn = gtk::Button::with_label("Cancelar");
    let apply_btn = gtk::Button::builder().label("Aplicar").build();
    apply_btn.add_css_class("suggested-action");
    apply_btn.add_css_class("pill");

    let actions_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .halign(gtk::Align::End)
        .build();
    actions_box.append(&cancel_btn);
    actions_box.append(&apply_btn);

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(16)
        .margin_top(16)
        .margin_bottom(16)
        .margin_start(16)
        .margin_end(16)
        .build();
    content.append(&group_files);
    content.append(&group_tone);
    content.append(&group_color);
    content.append(&group_out);
    content.append(&status);
    content.append(&actions_box);

    let scrolled = gtk::ScrolledWindow::builder()
        .hexpand(true)
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .child(&content)
        .build();
    scrolled.set_size_request(420, -1);

    // Two-column layout: scrollable form on the left, live preview column
    // on the right (when a session is available).
    let main_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(16)
        .margin_top(0)
        .margin_bottom(0)
        .build();
    main_box.append(&scrolled);

    let preview_picture: Option<gtk::Picture> = if let Some(sess) = preview_session.as_ref() {
        let (picture, column) = build_preview_column(sess);
        column.set_margin_top(16);
        column.set_margin_bottom(16);
        column.set_margin_end(16);
        main_box.append(&column);
        Some(picture)
    } else {
        None
    };

    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&header);
    toolbar_view.set_content(Some(&main_box));
    window.set_content(Some(&toolbar_view));

    // Wire live preview on every slider / gamma change.
    if let (Some(picture), Some(session)) = (preview_picture.as_ref(), preview_session.as_ref()) {
        let debounce = PreviewDebounce::new();
        let sliders: [adw::SpinRow; 4] =
            [brightness.clone(), contrast.clone(), saturation.clone(), gamma.clone()];
        for row in &sliders {
            let debounce = debounce.clone();
            let session = session.clone();
            let picture = picture.clone();
            let brightness = brightness.clone();
            let contrast = contrast.clone();
            let saturation = saturation.clone();
            let gamma = gamma.clone();
            row.connect_changed(move |_| {
                let ops = AdjustOps {
                    brightness: brightness.value() as i32,
                    contrast: contrast.value() as f32,
                    saturation: saturation.value() as f32,
                    gamma: gamma.value() as f32,
                };
                let session = session.clone();
                let picture = picture.clone();
                debounce.trigger(Duration::from_millis(80), move || {
                    refresh_preview(&session, &picture, &PreviewOp::Adjust(ops));
                });
            });
        }
    }

    {
        let window = window.clone();
        cancel_btn.connect_clicked(move |_| window.close());
    }

    {
        let files = files.clone();
        let brightness = brightness.clone();
        let contrast = contrast.clone();
        let saturation = saturation.clone();
        let gamma = gamma.clone();
        let target_row = target_row.clone();
        let overwrite_row = overwrite_row.clone();
        let status = status.clone();
        let apply_btn = apply_btn.clone();
        let cancel_btn = cancel_btn.clone();
        let window = window.clone();
        apply_btn.clone().connect_clicked(move |_| {
            let ops = AdjustOps {
                brightness: brightness.value() as i32,
                contrast: contrast.value() as f32,
                saturation: saturation.value() as f32,
                gamma: gamma.value() as f32,
            };
            let target: Option<Format> = match target_row.selected() {
                0 => None,
                n => Some(CONVERT_FORMATS[(n as usize) - 1].1),
            };
            let policy = OVERWRITE_CHOICES[overwrite_row.selected() as usize].1;

            apply_btn.set_sensitive(false);
            cancel_btn.set_sensitive(false);
            status.set_text("Ajustando…");

            let files = files.clone();
            let status = status.clone();
            let window = window.clone();
            let apply_btn = apply_btn.clone();
            let cancel_btn = cancel_btn.clone();
            glib::idle_add_local_once(move || {
                let (ok, skip, fail, first_err) = run_adjust_batch(&files, ops, target, policy);
                finish_dialog(&status, &apply_btn, &cancel_btn, &window, ok, skip, fail, first_err);
            });
        });
    }

    window
}

fn run_adjust_batch(
    files: &[PathBuf],
    ops: AdjustOps,
    target: Option<Format>,
    policy: OverwritePolicy,
) -> (usize, usize, usize, Option<String>) {
    let mut ok = 0;
    let mut skip = 0;
    let mut fail = 0;
    let mut first_err = None;
    for f in files {
        match adjust_file(f, ops, target, policy) {
            Ok(ConvertOutcome::Written { .. }) => ok += 1,
            Ok(ConvertOutcome::Skipped { .. }) => skip += 1,
            Err(e) => {
                fail += 1;
                if first_err.is_none() {
                    first_err = Some(format!("{}: {e}", f.display()));
                }
            }
        }
    }
    (ok, skip, fail, first_err)
}

// ---------------------------------------------------------------------------
// Metadata viewer dialog — read-only. Loads EXIF via `bigimage_core::metadata`
// and shows every tag as an AdwActionRow so users can scan + copy values.
// ---------------------------------------------------------------------------

/// Run the metadata viewer for `files[0]`. Multi-select still opens but
/// only the first file is inspected — a "próximo" navegator fica para M3.
pub fn run_metadata_dialog(files: Vec<PathBuf>) -> i32 {
    let app = adw::Application::builder().application_id(APP_ID).build();
    let files = Rc::new(files);
    app.connect_activate(move |app| {
        let Some(first) = files.first() else { return };
        build_properties_window(app, first).present();
    });
    let code = app.run_with_args::<&str>(&[]);
    i32::from(u8::from(code))
}

/// Janela de propriedades da imagem. Sempre traz as infos básicas do
/// arquivo (nome, pasta, tamanho, modificado, dimensões) mesmo sem EXIF —
/// é o que o usuário enxerga quando clica no "i". Se houver EXIF, uma
/// seção extra aparece abaixo com os campos crus.
fn build_properties_window(app: &adw::Application, path: &Path) -> adw::ApplicationWindow {
    let filename = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .default_width(520)
        .default_height(640)
        .title(format!("Propriedades — {filename}"))
        .build();

    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&adw::WindowTitle::new("Propriedades", &filename)));

    let page = adw::PreferencesPage::new();
    page.add(&build_properties_file_group(path));
    page.add(&build_properties_image_group(path));

    if let Ok(meta) = metadata::read(path) {
        if !meta.is_empty() {
            if meta.has_gps || meta.has_camera_info {
                let warning = adw::PreferencesGroup::builder().title("Privacidade").build();
                let row = adw::ActionRow::builder()
                    .title(if meta.has_gps {
                        "Coordenadas GPS presentes"
                    } else {
                        "Identificação de câmera/software presente"
                    })
                    .subtitle("Remova antes de publicar se for sensível.")
                    .build();
                warning.add(&row);
                page.add(&warning);
            }
            page.add(&build_properties_exif_group(&meta));
        }
    }

    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&header);
    toolbar_view.set_content(Some(&page));
    window.set_content(Some(&toolbar_view));

    window
}

fn build_properties_file_group(path: &Path) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder().title("Arquivo").build();

    let filename = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    group.add(&info_row("Nome", &filename));

    if let Some(parent) = path.parent() {
        let parent_str = parent.display().to_string();
        if !parent_str.is_empty() {
            group.add(&info_row("Pasta", &parent_str));
        }
    }

    if let Some(ext) = path.extension() {
        group.add(&info_row("Formato", &ext.to_string_lossy().to_uppercase()));
    }

    match std::fs::metadata(path) {
        Ok(fsmeta) => {
            group.add(&info_row("Tamanho", &format_bytes(fsmeta.len())));
            if let Ok(mtime) = fsmeta.modified() {
                group.add(&info_row("Modificado", &format_system_time(mtime)));
            }
        }
        Err(e) => {
            group.add(&info_row("Tamanho", &format!("(indisponível: {e})")));
        }
    }

    group
}

fn build_properties_image_group(path: &Path) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder().title("Imagem").build();
    match image::image_dimensions(path) {
        Ok((w, h)) => {
            group.add(&info_row("Dimensões", &format!("{w} × {h} px")));
            let mp = (f64::from(w) * f64::from(h)) / 1_000_000.0;
            if mp >= 0.1 {
                group.add(&info_row("Megapixels", &format!("{mp:.1} MP")));
            }
        }
        Err(e) => {
            group.add(&info_row("Dimensões", &format!("(falha: {e})")));
        }
    }
    // Orientação EXIF — só aparece se houver tag e != 1 (1 = upright).
    if let Ok(meta) = metadata::read(path) {
        if let Some(o) = meta.orientation {
            if o != 1 {
                group.add(&info_row(
                    "Orientação EXIF",
                    &format!("{o} · use Girar ▸ Automático pra corrigir"),
                ));
            }
        }
    }
    group
}

fn build_properties_exif_group(meta: &metadata::Metadata) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder()
        .title("EXIF")
        .description(format!("{} campo(s) presentes", meta.tags.len()))
        .build();
    for (name, value) in &meta.tags {
        group.add(&info_row(name, value));
    }
    group
}

/// ActionRow só-leitura com título+valor. Padrão visual usado no dialog
/// de propriedades pra cada campo.
fn info_row(title: &str, value: &str) -> adw::ActionRow {
    adw::ActionRow::builder().title(title).subtitle(value).build()
}

/// Formata bytes pra uma escala humana (KB/MB/GB em base 1024).
fn format_bytes(n: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut v = n as f64;
    let mut u = 0usize;
    while v >= 1024.0 && u + 1 < UNITS.len() {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {} ({n} bytes)", UNITS[u])
    }
}

/// Formata um SystemTime em `YYYY-MM-DD HH:MM` local. Usa apenas a API
/// padrão (sem pegar `chrono`) — basta pra mostrar a data legível.
fn format_system_time(t: std::time::SystemTime) -> String {
    let secs = t.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    // glib::DateTime lida com timezone local automaticamente — usamos ela
    // pra evitar reimplementar aritmética de calendário.
    match glib::DateTime::from_unix_local(secs) {
        Ok(dt) => {
            dt.format("%Y-%m-%d %H:%M").map(|g| g.to_string()).unwrap_or_else(|_| format!("{secs}"))
        }
        Err(_) => format!("{secs}"),
    }
}

// ---------------------------------------------------------------------------
// Compare dialog — two images in a `gtk::Paned`, drag the divider to sweep
// between "antes" and "depois". Requires exactly 2 files; the rest are
// ignored (first two in the selection are used).
// ---------------------------------------------------------------------------

/// Open the "Compare two" window. Needs two files — if fewer are given
/// we fall back to the viewer with whatever's there.
pub fn run_compare_dialog(files: Vec<PathBuf>) -> i32 {
    if files.len() < 2 {
        tracing::warn!(count = files.len(), "compare needs two files; falling back to viewer");
        return run_viewer(files);
    }
    let app = adw::Application::builder().application_id(APP_ID).build();
    let files = Rc::new(files);
    app.connect_activate(move |app| {
        build_compare_dialog(app, files.clone()).present();
    });
    let code = app.run_with_args::<&str>(&[]);
    i32::from(u8::from(code))
}

fn build_compare_dialog(app: &adw::Application, files: Rc<Vec<PathBuf>>) -> adw::ApplicationWindow {
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .default_width(1200)
        .default_height(720)
        .title("Comparar imagens — Íris")
        .build();

    let left_path = &files[0];
    let right_path = &files[1];

    let header = adw::HeaderBar::new();
    let left_name =
        left_path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let right_name =
        right_path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let title = adw::WindowTitle::new(
        "Comparar imagens",
        &format!("Antes: {left_name}  ·  Depois: {right_name}"),
    );
    header.set_title_widget(Some(&title));

    // Captions gone — titlebar now carries the Antes/Depois info so the
    // full vertical space goes to the pictures.
    let left_picture = make_compare_side(left_path);
    let right_picture = make_compare_side(right_path);

    let paned = gtk::Paned::builder()
        .orientation(gtk::Orientation::Horizontal)
        .hexpand(true)
        .vexpand(true)
        .resize_start_child(true)
        .resize_end_child(true)
        .shrink_start_child(false)
        .shrink_end_child(false)
        .start_child(&left_picture)
        .end_child(&right_picture)
        .build();

    let hint = gtk::Label::builder()
        .label("Arraste o divisor para comparar · Esc fecha")
        .xalign(0.5)
        .margin_top(6)
        .margin_bottom(6)
        .build();
    hint.add_css_class("dim-label");

    let content = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(0).build();
    content.append(&paned);
    content.append(&hint);

    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&header);
    toolbar_view.set_content(Some(&content));
    window.set_content(Some(&toolbar_view));

    // Position the divider at 50% once the window has real dimensions.
    {
        let paned = paned.clone();
        let window = window.clone();
        glib::idle_add_local_once(move || {
            let w = window.default_width();
            if w > 0 {
                paned.set_position(w / 2);
            }
        });
    }

    // Keyboard: Esc closes.
    let key = gtk::EventControllerKey::new();
    {
        let window = window.clone();
        key.connect_key_pressed(move |_, k, _, _| {
            if matches!(k, gdk::Key::Escape) {
                window.close();
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
    }
    window.add_controller(key);

    window
}

fn make_compare_side(path: &Path) -> gtk::Box {
    let picture = gtk::Picture::new();
    picture.set_content_fit(gtk::ContentFit::Contain);
    picture.set_hexpand(true);
    picture.set_vexpand(true);

    match load_texture(path) {
        Ok((tex, _, _)) => picture.set_paintable(Some(&tex)),
        Err(e) => {
            tracing::warn!(?path, error = %e, "compare: failed to load");
        }
    }

    let column = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(0)
        .hexpand(true)
        .vexpand(true)
        .build();
    column.append(&picture);
    column
}

// ---------------------------------------------------------------------------
// Animate dialog — pick a delay, a loop mode, an output path and build an
// animated GIF from the selected frames. Minimal shape: every parameter
// has a sensible default so the user can just hit "Criar GIF" and get
// something playable.
// ---------------------------------------------------------------------------

/// Open the "Criar GIF animado" modal. Needs 2+ frames; falls back to an
/// error page when fewer.
/// Progress state for the batch dialog — updated per-file by the
/// idle-driven loop and read by the UI to paint the progress bar.
struct BatchProgress {
    total: usize,
    done: usize,
    ok: usize,
    skip: usize,
    fail: usize,
    first_err: Option<String>,
    cancelled: bool,
}

/// Prisma Lote — converter em lote. Pega a lista de arquivos recebida via
/// CLI (hoje) e aplica a conversão escolhida um por um, com barra de
/// progresso e botão Cancelar. MVP: só conversão + qualidade + overwrite.
/// Próximas rodadas: resize + rotate combinados, picker de pasta destino,
/// adicionar/remover arquivos dentro do próprio diálogo.
pub fn run_batch_dialog(files: Vec<PathBuf>) -> i32 {
    let app = adw::Application::builder().application_id(APP_ID).build();
    let files = Rc::new(RefCell::new(files));
    app.connect_activate(move |app| {
        build_batch_dialog(app, files.clone()).present();
    });
    let code = app.run_with_args::<&str>(&[]);
    i32::from(u8::from(code))
}

fn build_batch_dialog(
    app: &adw::Application,
    files: Rc<RefCell<Vec<PathBuf>>>,
) -> adw::ApplicationWindow {
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .default_width(640)
        .default_height(640)
        .title("Prisma — Lote")
        .build();

    let header = adw::HeaderBar::new();
    let title = adw::WindowTitle::new("Conversão em lote", "Prisma");
    header.set_title_widget(Some(&title));

    let format_names: Vec<&str> = CONVERT_FORMATS.iter().map(|(n, _)| *n).collect();
    let overwrite_names: Vec<&str> = OVERWRITE_CHOICES.iter().map(|(n, _)| *n).collect();

    // ── Grupo: arquivos ─────────────────────────────────────────────────
    // Lista mutável (compartilhada via Rc<RefCell>) — suporta adicionar
    // arquivos e pastas depois do diálogo aberto. `summary_row` é
    // re-renderizada sempre que mudar a lista, via `refresh_files_row`.
    let files_group = adw::PreferencesGroup::builder().title("Arquivos").build();
    let summary_row = adw::ActionRow::builder().activatable(false).build();
    let add_files_btn = gtk::Button::builder()
        .icon_name("list-add-symbolic")
        .tooltip_text("Adicionar arquivos")
        .css_classes(["flat"])
        .valign(gtk::Align::Center)
        .build();
    let add_folder_btn = gtk::Button::builder()
        .icon_name("folder-symbolic")
        .tooltip_text("Adicionar pasta (imagens)")
        .css_classes(["flat"])
        .valign(gtk::Align::Center)
        .build();
    summary_row.add_suffix(&add_files_btn);
    summary_row.add_suffix(&add_folder_btn);
    files_group.add(&summary_row);
    refresh_files_row(&summary_row, &files.borrow());

    // ── Grupo: conversão ────────────────────────────────────────────────
    let convert_group = adw::PreferencesGroup::builder().title("Conversão").build();

    let format_row = adw::ComboRow::builder()
        .title("Formato de destino")
        .model(&gtk::StringList::new(&format_names))
        .build();
    convert_group.add(&format_row);

    let quality_spin = adw::SpinRow::with_range(1.0, 100.0, 1.0);
    quality_spin.set_title("Qualidade (JPEG/lossy)");
    quality_spin.set_subtitle("1 pior · 85 bom · 100 sem perdas");
    quality_spin.set_value(85.0);
    convert_group.add(&quality_spin);

    let progressive_switch = adw::SwitchRow::builder()
        .title("JPEG progressivo")
        .subtitle("carregamento em passes (formatos que suportam)")
        .build();
    convert_group.add(&progressive_switch);

    let optimize_switch = adw::SwitchRow::builder()
        .title("Otimizar tamanho")
        .subtitle("PNG com compressão máxima; mais lento, arquivo menor")
        .build();
    convert_group.add(&optimize_switch);

    // ── Grupo: destino ──────────────────────────────────────────────────
    let dest_group = adw::PreferencesGroup::builder().title("Destino").build();
    let overwrite_row = adw::ComboRow::builder()
        .title("Se o arquivo de saída existir")
        .model(&gtk::StringList::new(&overwrite_names))
        .build();
    dest_group.add(&overwrite_row);
    // Pasta de saída: None = mesma do original. Clicar abre FolderDialog.
    let output_dir: Rc<RefCell<Option<PathBuf>>> = Rc::new(RefCell::new(None));
    let folder_row = adw::ActionRow::builder()
        .title("Pasta de saída")
        .subtitle("Mesma pasta do arquivo de origem")
        .activatable(true)
        .build();
    let folder_icon = gtk::Image::from_icon_name("folder-open-symbolic");
    folder_icon.add_css_class("dim-label");
    folder_row.add_suffix(&folder_icon);
    let clear_folder_btn = gtk::Button::builder()
        .icon_name("edit-clear-symbolic")
        .tooltip_text("Usar a mesma pasta do arquivo de origem")
        .css_classes(["flat"])
        .valign(gtk::Align::Center)
        .visible(false)
        .build();
    folder_row.add_suffix(&clear_folder_btn);
    dest_group.add(&folder_row);

    // ── Rodapé: barra de progresso + status + botões ────────────────────
    let progress = gtk::ProgressBar::builder().show_text(true).text("Pronto").build();
    progress.set_margin_start(18);
    progress.set_margin_end(18);
    progress.set_margin_top(8);

    let status = gtk::Label::builder()
        .label("Aguardando. Clique em Processar para iniciar.")
        .wrap(true)
        .max_width_chars(64)
        .css_classes(["dim-label"])
        .margin_start(18)
        .margin_end(18)
        .margin_top(4)
        .margin_bottom(4)
        .build();

    let apply_btn =
        gtk::Button::builder().label("Processar").css_classes(["pill", "suggested-action"]).build();
    let cancel_btn = gtk::Button::builder().label("Cancelar").build();
    let btn_bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    btn_bar.set_halign(gtk::Align::Center);
    btn_bar.set_margin_top(8);
    btn_bar.set_margin_bottom(18);
    btn_bar.append(&cancel_btn);
    btn_bar.append(&apply_btn);

    // ── Scroll wrapper (para listas longas no futuro) ───────────────────
    let options_col = gtk::Box::new(gtk::Orientation::Vertical, 14);
    options_col.set_margin_start(18);
    options_col.set_margin_end(18);
    options_col.set_margin_top(18);
    options_col.append(&files_group);
    options_col.append(&convert_group);
    options_col.append(&dest_group);

    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&options_col)
        .build();

    let main_col = gtk::Box::new(gtk::Orientation::Vertical, 0);
    main_col.append(&scroller);
    main_col.append(&progress);
    main_col.append(&status);
    main_col.append(&btn_bar);

    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&header);
    toolbar_view.set_content(Some(&main_col));
    window.set_content(Some(&toolbar_view));

    // Estado partilhado entre UI e worker de progresso. RefCell em Rc é
    // suficiente — processamos no main thread um arquivo por idle tick.
    let progress_state: Rc<RefCell<BatchProgress>> = Rc::new(RefCell::new(BatchProgress {
        total: files.borrow().len(),
        done: 0,
        ok: 0,
        skip: 0,
        fail: 0,
        first_err: None,
        cancelled: false,
    }));

    // Handlers dos botões "Adicionar arquivos" e "Adicionar pasta".
    {
        let files = files.clone();
        let summary_row = summary_row.clone();
        let window = window.clone();
        add_files_btn.connect_clicked(move |_| {
            add_files_to_batch(&window, &files, &summary_row);
        });
    }
    {
        let files = files.clone();
        let summary_row = summary_row.clone();
        let window = window.clone();
        add_folder_btn.connect_clicked(move |_| {
            add_folder_to_batch(&window, &files, &summary_row);
        });
    }

    // Pasta de saída: clicar na row abre FolderDialog; o botão vassoura
    // reseta para None (mesma pasta do original).
    {
        let output_dir = output_dir.clone();
        let folder_row = folder_row.clone();
        let clear_btn = clear_folder_btn.clone();
        let window = window.clone();
        folder_row.clone().connect_activated(move |_| {
            pick_output_folder(&window, &output_dir, &folder_row, &clear_btn);
        });
    }
    {
        let output_dir = output_dir.clone();
        let folder_row = folder_row.clone();
        let clear_btn = clear_folder_btn.clone();
        clear_folder_btn.connect_clicked(move |_| {
            *output_dir.borrow_mut() = None;
            folder_row.set_subtitle("Mesma pasta do arquivo de origem");
            clear_btn.set_visible(false);
        });
    }

    {
        let state = progress_state.clone();
        let window = window.clone();
        cancel_btn.connect_clicked(move |_| {
            let is_running = {
                let s = state.borrow();
                s.done < s.total && !s.cancelled
            };
            if is_running {
                state.borrow_mut().cancelled = true;
            } else {
                window.close();
            }
        });
    }

    {
        let files = files.clone();
        let format_row = format_row.clone();
        let overwrite_row = overwrite_row.clone();
        let quality_spin = quality_spin.clone();
        let progressive_switch = progressive_switch.clone();
        let optimize_switch = optimize_switch.clone();
        let progress = progress.clone();
        let status = status.clone();
        let apply_btn = apply_btn.clone();
        let cancel_btn = cancel_btn.clone();
        let window = window.clone();
        let progress_state = progress_state.clone();
        let output_dir = output_dir.clone();
        apply_btn.clone().connect_clicked(move |_| {
            let fmt_idx = format_row.selected() as usize;
            let policy_idx = overwrite_row.selected() as usize;
            let fmt = CONVERT_FORMATS[fmt_idx].1;
            let policy = OVERWRITE_CHOICES[policy_idx].1;
            let opts = EncodeOptions {
                quality: Some(quality_spin.value() as u8),
                progressive: progressive_switch.is_active(),
                optimize: optimize_switch.is_active(),
            };
            let out_dir_snapshot = output_dir.borrow().clone();

            // Reset state for a re-run; total recomputed from the live list.
            {
                let mut s = progress_state.borrow_mut();
                s.total = files.borrow().len();
                s.done = 0;
                s.ok = 0;
                s.skip = 0;
                s.fail = 0;
                s.first_err = None;
                s.cancelled = false;
            }
            apply_btn.set_sensitive(false);
            progress.set_fraction(0.0);
            progress.set_text(Some("Iniciando…"));
            status.set_text("Processando…");

            // Processa 1 arquivo por idle tick — UI continua responsiva,
            // Cancelar efetivo a cada iteração. Não é thread-pool (single
            // threaded), mas é correto e simples para o MVP.
            let files = files.clone();
            let progress = progress.clone();
            let status = status.clone();
            let apply_btn = apply_btn.clone();
            let cancel_btn = cancel_btn.clone();
            let window = window.clone();
            let progress_state = progress_state.clone();
            glib::idle_add_local(move || {
                let (idx, cancelled) = {
                    let s = progress_state.borrow();
                    (s.done, s.cancelled)
                };
                let total = files.borrow().len();
                if cancelled || idx >= total {
                    let s = progress_state.borrow();
                    let msg = if s.cancelled {
                        format!(
                            "Cancelado após {}/{}. {} gravado(s), {} ignorado(s), {} falha(s).",
                            s.done, s.total, s.ok, s.skip, s.fail
                        )
                    } else {
                        format!(
                            "Concluído. {} gravado(s), {} ignorado(s), {} falha(s).",
                            s.ok, s.skip, s.fail
                        )
                    };
                    status.set_text(&msg);
                    if let Some(err) = &s.first_err {
                        status.set_text(&format!("{msg}\n{err}"));
                    }
                    progress.set_text(Some(if s.cancelled { "Cancelado" } else { "Concluído" }));
                    apply_btn.set_sensitive(true);
                    cancel_btn.set_label(if s.done == s.total && !s.cancelled {
                        "Fechar"
                    } else {
                        "Cancelar"
                    });
                    // Auto-close em sucesso limpo
                    if !s.cancelled && s.fail == 0 {
                        let win = window.clone();
                        glib::timeout_add_seconds_local_once(3, move || win.close());
                    }
                    return glib::ControlFlow::Break;
                }

                let file = files.borrow()[idx].clone();
                let display_name = file
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| file.display().to_string());
                progress.set_text(Some(&format!("{}/{} · {}", idx + 1, total, display_name)));

                let result =
                    convert_file_to(&file, out_dir_snapshot.as_deref(), fmt, &opts, policy);
                match result {
                    Ok(ConvertOutcome::Written { .. }) => progress_state.borrow_mut().ok += 1,
                    Ok(ConvertOutcome::Skipped { .. }) => progress_state.borrow_mut().skip += 1,
                    Err(e) => {
                        let mut s = progress_state.borrow_mut();
                        s.fail += 1;
                        if s.first_err.is_none() {
                            s.first_err = Some(format!("{}: {e}", file.display()));
                        }
                    }
                }
                let mut s = progress_state.borrow_mut();
                s.done += 1;
                progress.set_fraction(s.done as f64 / s.total.max(1) as f64);
                glib::ControlFlow::Continue
            });
        });
    }

    window
}

/// Atualiza a ActionRow de resumo do Prisma Lote com a contagem + peso
/// total da lista atual de arquivos.
fn refresh_files_row(row: &adw::ActionRow, files: &[PathBuf]) {
    let total: u64 = files.iter().filter_map(|p| std::fs::metadata(p).ok().map(|m| m.len())).sum();
    row.set_title(&format!("{} arquivo(s) selecionado(s)", files.len()));
    row.set_subtitle(&format_size(total));
}

/// Abre um picker multi-arquivo e anexa os escolhidos à lista do batch,
/// deduplicando pelas paths canônicas. Formatos filtrados pelos mesmos
/// MIME types que o dashboard aceita.
fn add_files_to_batch(
    window: &adw::ApplicationWindow,
    files: &Rc<RefCell<Vec<PathBuf>>>,
    row: &adw::ActionRow,
) {
    let dialog = gtk::FileDialog::builder().title("Adicionar arquivos").modal(true).build();
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("Imagens"));
    for mime in [
        "image/jpeg",
        "image/png",
        "image/webp",
        "image/avif",
        "image/gif",
        "image/tiff",
        "image/bmp",
        "image/heic",
        "image/heif",
        "image/jxl",
    ] {
        filter.add_mime_type(mime);
    }
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    dialog.set_filters(Some(&filters));

    let parent = window.clone();
    let files = files.clone();
    let row = row.clone();
    dialog.open_multiple(Some(&parent), None::<&gio::Cancellable>, move |result| {
        let Ok(list) = result else { return };
        let paths: Vec<PathBuf> = (0..list.n_items())
            .filter_map(|i| list.item(i))
            .filter_map(|o| o.downcast::<gio::File>().ok())
            .filter_map(|f| f.path())
            .collect();
        if paths.is_empty() {
            return;
        }
        append_unique(&files, paths);
        refresh_files_row(&row, &files.borrow());
    });
}

/// Abre um picker de pasta e varre (não-recursivo, um nível só) em busca
/// de arquivos com extensão de imagem conhecida; anexa à lista do batch.
fn add_folder_to_batch(
    window: &adw::ApplicationWindow,
    files: &Rc<RefCell<Vec<PathBuf>>>,
    row: &adw::ActionRow,
) {
    let dialog =
        gtk::FileDialog::builder().title("Adicionar pasta com imagens").modal(true).build();
    let parent = window.clone();
    let files = files.clone();
    let row = row.clone();
    dialog.select_folder(Some(&parent), None::<&gio::Cancellable>, move |result| {
        let Ok(folder) = result else { return };
        let Some(dir) = folder.path() else { return };
        let found = collect_images_in_dir(&dir);
        if found.is_empty() {
            return;
        }
        append_unique(&files, found);
        refresh_files_row(&row, &files.borrow());
    });
}

/// Adiciona novos paths à lista, pulando os que já estavam lá
/// (comparação por PathBuf canônica).
fn append_unique(files: &Rc<RefCell<Vec<PathBuf>>>, new_paths: Vec<PathBuf>) {
    let mut list = files.borrow_mut();
    for p in new_paths {
        if !list.iter().any(|existing| existing == &p) {
            list.push(p);
        }
    }
}

/// Extensões que aceitamos numa varredura de pasta. Case-insensitive.
const IMAGE_EXTENSIONS: &[&str] =
    &["jpg", "jpeg", "png", "webp", "avif", "gif", "tif", "tiff", "bmp", "heic", "heif", "jxl"];

fn collect_images_in_dir(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(ext) = path.extension() else { continue };
        let ext_lower = ext.to_string_lossy().to_ascii_lowercase();
        if IMAGE_EXTENSIONS.iter().any(|e| *e == ext_lower) {
            out.push(path);
        }
    }
    out.sort();
    out
}

/// Abre FolderDialog; ao confirmar, atualiza `output_dir` e o subtitle
/// da row. Revela o botão "limpar" para o usuário voltar ao padrão.
fn pick_output_folder(
    window: &adw::ApplicationWindow,
    output_dir: &Rc<RefCell<Option<PathBuf>>>,
    row: &adw::ActionRow,
    clear_btn: &gtk::Button,
) {
    let dialog = gtk::FileDialog::builder().title("Escolher pasta de saída").modal(true).build();
    let parent = window.clone();
    let output_dir = output_dir.clone();
    let row = row.clone();
    let clear_btn = clear_btn.clone();
    dialog.select_folder(Some(&parent), None::<&gio::Cancellable>, move |result| {
        let Ok(folder) = result else { return };
        let Some(path) = folder.path() else { return };
        row.set_subtitle(&path.display().to_string());
        *output_dir.borrow_mut() = Some(path);
        clear_btn.set_visible(true);
    });
}

/// Humaniza um tamanho em bytes para algo como "2.3 GB" ou "842 KB".
fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.0} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

pub fn run_animate_dialog(files: Vec<PathBuf>) -> i32 {
    let app = adw::Application::builder().application_id(APP_ID).build();
    let files = Rc::new(files);
    app.connect_activate(move |app| {
        build_animate_dialog(app, files.clone()).present();
    });
    let code = app.run_with_args::<&str>(&[]);
    i32::from(u8::from(code))
}

fn build_animate_dialog(app: &adw::Application, files: Rc<Vec<PathBuf>>) -> adw::ApplicationWindow {
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .default_width(520)
        .default_height(520)
        .title("Criar GIF animado — Prisma")
        .build();

    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&adw::WindowTitle::new("Criar GIF animado", "Prisma")));

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(16)
        .margin_top(16)
        .margin_bottom(16)
        .margin_start(16)
        .margin_end(16)
        .build();

    if files.len() < 2 {
        let status = adw::StatusPage::builder()
            .icon_name("dialog-warning-symbolic")
            .title("Selecione pelo menos 2 imagens")
            .description("Informe vários arquivos de uma vez para montar a animação.")
            .vexpand(true)
            .build();
        content.append(&status);

        let close = gtk::Button::with_label("Fechar");
        {
            let window = window.clone();
            close.connect_clicked(move |_| window.close());
        }
        content.append(&close);

        let tv = adw::ToolbarView::new();
        tv.add_top_bar(&header);
        tv.set_content(Some(&content));
        window.set_content(Some(&tv));
        return window;
    }

    // Parametric rows.
    let delay_spin = adw::SpinRow::with_range(10.0, 5000.0, 10.0);
    delay_spin.set_title("Atraso por quadro (ms)");
    delay_spin.set_subtitle("100 ≈ 10 fps · 40 ≈ 25 fps · 1000 = 1 s");
    delay_spin.set_value(100.0);

    let loop_row = adw::ComboRow::builder()
        .title("Loop")
        .model(&gtk::StringList::new(&[
            "Infinito (padrão)",
            "Uma vez",
            "2 vezes",
            "5 vezes",
            "10 vezes",
        ]))
        .build();

    let speed_spin = adw::SpinRow::with_range(1.0, 30.0, 1.0);
    speed_spin.set_title("Velocidade do encoder");
    speed_spin.set_subtitle("maior = mais rápido, paleta um pouco pior · padrão 10");
    speed_spin.set_value(10.0);

    // Default output: next to the first frame, named "animated.gif".
    let default_output: PathBuf = files[0]
        .parent()
        .map(|p| p.join("animated.gif"))
        .unwrap_or_else(|| PathBuf::from("animated.gif"));
    let output_row = adw::EntryRow::builder().title("Arquivo de saída (.gif)").build();
    output_row.set_text(&default_output.to_string_lossy());

    let files_row = adw::ActionRow::builder()
        .title(format!("{} quadro(s)", files.len()))
        .subtitle(files_subtitle(&files))
        .build();

    let group = adw::PreferencesGroup::new();
    group.add(&files_row);
    group.add(&delay_spin);
    group.add(&loop_row);
    group.add(&speed_spin);
    group.add(&output_row);
    content.append(&group);

    let status = gtk::Label::builder().label("").wrap(true).xalign(0.0).build();
    status.add_css_class("dim-label");
    content.append(&status);

    let cancel_btn = gtk::Button::with_label("Cancelar");
    let apply_btn = gtk::Button::builder().label("Criar GIF").build();
    apply_btn.add_css_class("suggested-action");
    apply_btn.add_css_class("pill");

    let actions_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .halign(gtk::Align::End)
        .build();
    actions_box.append(&cancel_btn);
    actions_box.append(&apply_btn);
    content.append(&actions_box);

    let tv = adw::ToolbarView::new();
    tv.add_top_bar(&header);
    tv.set_content(Some(&content));
    window.set_content(Some(&tv));

    {
        let window = window.clone();
        cancel_btn.connect_clicked(move |_| window.close());
    }

    {
        let files = files.clone();
        let delay_spin = delay_spin.clone();
        let loop_row = loop_row.clone();
        let speed_spin = speed_spin.clone();
        let output_row = output_row.clone();
        let status = status.clone();
        let apply_btn = apply_btn.clone();
        let cancel_btn = cancel_btn.clone();
        let window = window.clone();
        apply_btn.clone().connect_clicked(move |_| {
            let loop_mode = match loop_row.selected() {
                0 => LoopMode::Infinite,
                1 => LoopMode::Once,
                2 => LoopMode::Finite(2),
                3 => LoopMode::Finite(5),
                _ => LoopMode::Finite(10),
            };
            let opts = AnimateOptions {
                delay_ms: delay_spin.value() as u32,
                loop_mode,
                speed: speed_spin.value() as i32,
            };
            let output: PathBuf = output_row.text().as_str().into();

            apply_btn.set_sensitive(false);
            cancel_btn.set_sensitive(false);
            status.set_text("Montando GIF…");

            let files = files.clone();
            let status = status.clone();
            let window = window.clone();
            let apply_btn = apply_btn.clone();
            let cancel_btn = cancel_btn.clone();
            glib::idle_add_local_once(move || {
                let result = make_gif(&files, &output, opts);
                match result {
                    Ok(path) => {
                        status.set_text(&format!("Pronto: {}", path.display()));
                        let w = window.clone();
                        glib::timeout_add_seconds_local_once(2, move || w.close());
                    }
                    Err(e) => {
                        status.set_text(&format!("Falha: {e}"));
                        apply_btn.set_sensitive(true);
                        cancel_btn.set_sensitive(true);
                    }
                }
            });
        });
    }

    window
}

/// Output path for `remove_background` — always PNG to preserve alpha.
/// Sibling of the source, stem suffixed with `_nobg`.
fn nobg_output_path(src: &Path) -> PathBuf {
    let stem = src.file_stem().and_then(|s| s.to_str()).unwrap_or("output");
    let parent = src.parent().unwrap_or_else(|| Path::new("."));
    parent.join(format!("{stem}_nobg.png"))
}

/// Run one remove-bg pass over `src`: decode → BiRefNet → save PNG RGBA.
/// `progress` receives the [`BgStage`] signals emitted by the backend so
/// dialogs can drive a `gtk::ProgressBar`. Errors bubble the reason so
/// the caller can show a precise message.
fn remove_bg_one_file_with_progress(
    src: &Path,
    progress: impl FnMut(bigimage_ai::background::BgStage),
) -> Result<PathBuf, String> {
    let img = image::open(src).map_err(|e| format!("decode: {e}"))?;
    let out = bigimage_ai::background::remove_background_with_progress(&img, progress)
        .map_err(|e| e.to_string())?;
    let dest = nobg_output_path(src);
    out.save_with_format(&dest, image::ImageFormat::Png).map_err(|e| format!("encode: {e}"))?;
    Ok(dest)
}

/// Worker-thread state for the remove-bg dialog. Shared via `Arc<Mutex>`
/// with a `glib::timeout_add_local` on the UI thread, which polls every
/// 100 ms and reflects the latest snapshot into the ProgressBar + label.
#[derive(Clone, Debug)]
enum BgUiState {
    /// Worker hasn't started yet (apply not clicked).
    Idle,
    /// Downloading the BiRefNet weights. `done`/`total` in bytes.
    Download { done: u64, total: u64 },
    /// Running inference on file `idx` (0-based) of `total`.
    Processing { idx: usize, total: usize, file: String },
    /// Worker finished. `outputs` are per-input written paths (or None
    /// for failed inputs — lines up with the originals by index).
    Done { outputs: Vec<Option<PathBuf>>, first_err: Option<String> },
}

pub fn run_remove_bg_dialog(files: Vec<PathBuf>) -> i32 {
    let app = adw::Application::builder().application_id(APP_ID).build();
    let files = Rc::new(files);
    app.connect_activate(move |app| {
        build_remove_bg_dialog(app, files.clone()).present();
    });
    let code = app.run_with_args::<&str>(&[]);
    i32::from(u8::from(code))
}

fn build_remove_bg_dialog(
    app: &adw::Application,
    files: Rc<Vec<PathBuf>>,
) -> adw::ApplicationWindow {
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .default_width(520)
        .default_height(420)
        .title("Remover fundo — Prisma")
        .build();

    let header = adw::HeaderBar::new();
    header
        .set_title_widget(Some(&adw::WindowTitle::new("Remover fundo", "Prisma · BiRefNet-lite")));

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(16)
        .margin_bottom(16)
        .margin_start(16)
        .margin_end(16)
        .build();

    // Empty input — fail closed with clear message.
    if files.is_empty() {
        let status = adw::StatusPage::builder()
            .icon_name("dialog-warning-symbolic")
            .title("Nenhuma imagem selecionada")
            .description("Informe um ou mais arquivos ao abrir o diálogo.")
            .vexpand(true)
            .build();
        content.append(&status);

        let close = gtk::Button::with_label("Fechar");
        {
            let window = window.clone();
            close.connect_clicked(move |_| window.close());
        }
        content.append(&close);

        let tv = adw::ToolbarView::new();
        tv.add_top_bar(&header);
        tv.set_content(Some(&content));
        window.set_content(Some(&tv));
        return window;
    }

    // Soft-fail build warning: if the `ai` feature is off, inference will
    // refuse anyway — but tell the user upfront so they don't click and
    // get a surprise error.
    if !bigimage_ai::onnx_available() {
        let banner = gtk::Label::builder()
            .label(
                "Esta build não inclui IA. Recompile com `cargo build --features ai` \
                 e instale `onnxruntime` (Arch: `pacman -S onnxruntime`).",
            )
            .wrap(true)
            .xalign(0.0)
            .build();
        banner.add_css_class("warning");
        content.append(&banner);
    }

    let files_row = adw::ActionRow::builder()
        .title(format!("{} imagem(ns)", files.len()))
        .subtitle(files_subtitle(&files))
        .build();
    let files_icon = gtk::Image::from_icon_name("image-x-generic-symbolic");
    files_row.add_prefix(&files_icon);

    let model_row = adw::ActionRow::builder()
        .title("Modelo")
        .subtitle("BiRefNet-lite (MIT) · roda offline no seu computador")
        .build();
    let model_icon = gtk::Image::from_icon_name("application-x-executable-symbolic");
    model_row.add_prefix(&model_icon);

    // Subtitle é Pango-markup em adw::ActionRow → angle-brackets ("<orig>")
    // eram interpretados como tag malformada e a linha ficava vazia.
    // Usar chevrons Unicode evita o parser sem escaping manual.
    let output_row = adw::ActionRow::builder()
        .title("Saída")
        .subtitle("«origem»_nobg.png · PNG com alfa, sem sobrescrever")
        .build();
    let output_icon = gtk::Image::from_icon_name("document-save-symbolic");
    output_row.add_prefix(&output_icon);

    let group = adw::PreferencesGroup::builder()
        .title("Remover fundo com IA local")
        .description(
            "O modelo roda inteiramente no seu computador. \
             Nenhuma imagem sai do disco — sem upload, sem conta, sem API externa.",
        )
        .build();
    group.add(&files_row);
    group.add(&model_row);
    group.add(&output_row);
    content.append(&group);

    // Progress bar — hidden until the user clicks Remover fundo. Shows
    // download bytes on first run, then per-file inference progress.
    let progress_bar =
        gtk::ProgressBar::builder().show_text(true).visible(false).margin_top(6).build();
    content.append(&progress_bar);

    let status = gtk::Label::builder().label("").wrap(true).xalign(0.0).build();
    status.add_css_class("dim-label");
    content.append(&status);

    let cancel_btn = gtk::Button::with_label("Cancelar");
    let apply_btn = gtk::Button::builder().label("Remover fundo").build();
    apply_btn.add_css_class("suggested-action");
    apply_btn.add_css_class("pill");

    let actions_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .halign(gtk::Align::End)
        .build();
    actions_box.append(&cancel_btn);
    actions_box.append(&apply_btn);
    content.append(&actions_box);

    let tv = adw::ToolbarView::new();
    tv.add_top_bar(&header);
    tv.set_content(Some(&content));
    window.set_content(Some(&tv));

    // Cancellation flag — shared between UI, worker thread, and the
    // close_request handler so fechar a janela (X, Esc, botão Cancelar)
    // efetivamente encerra a interação: o Compare pós-sucesso NÃO abre,
    // e o worker para de processar arquivos restantes.
    let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

    {
        let cancelled = cancelled.clone();
        window.connect_close_request(move |_| {
            cancelled.store(true, std::sync::atomic::Ordering::Relaxed);
            glib::Propagation::Proceed
        });
    }
    {
        let window = window.clone();
        cancel_btn.connect_clicked(move |_| window.close());
    }

    {
        let files_rc = files.clone();
        let status = status.clone();
        let progress_bar = progress_bar.clone();
        let apply_btn = apply_btn.clone();
        let cancel_btn = cancel_btn.clone();
        let window = window.clone();
        let cancelled = cancelled.clone();
        apply_btn.clone().connect_clicked(move |_| {
            apply_btn.set_sensitive(false);
            // Cancelar continua disponível — vira "abortar e fechar".
            cancel_btn.set_label("Cancelar");
            progress_bar.set_visible(true);
            progress_bar.set_fraction(0.0);
            progress_bar.set_text(Some("Preparando…"));
            status.set_text("");

            // Rc<Vec<PathBuf>> não é Send — worker recebe um Vec próprio.
            let files_owned: Vec<PathBuf> = (*files_rc).clone();
            let shared: std::sync::Arc<std::sync::Mutex<BgUiState>> =
                std::sync::Arc::new(std::sync::Mutex::new(BgUiState::Idle));

            {
                let shared = shared.clone();
                let cancelled_w = cancelled.clone();
                std::thread::spawn(move || {
                    let total = files_owned.len();
                    let mut outputs: Vec<Option<PathBuf>> = Vec::with_capacity(total);
                    let mut first_err: Option<String> = None;
                    for (idx, path) in files_owned.iter().enumerate() {
                        // Bail entre arquivos — não dá para interromper o
                        // session.run() do ORT no meio, mas pelo menos o
                        // próximo arquivo do lote não é tocado.
                        if cancelled_w.load(std::sync::atomic::Ordering::Relaxed) {
                            outputs.push(None);
                            continue;
                        }
                        {
                            let mut st = shared.lock().unwrap();
                            *st = BgUiState::Processing {
                                idx,
                                total,
                                file: path
                                    .file_name()
                                    .map(|n| n.to_string_lossy().into_owned())
                                    .unwrap_or_default(),
                            };
                        }
                        let shared_cb = shared.clone();
                        let result = remove_bg_one_file_with_progress(path, move |stage| {
                            if let bigimage_ai::background::BgStage::Download { done, total } =
                                stage
                            {
                                let mut st = shared_cb.lock().unwrap();
                                *st = BgUiState::Download { done, total };
                            }
                        });
                        match result {
                            Ok(out) => outputs.push(Some(out)),
                            Err(e) => {
                                if first_err.is_none() {
                                    first_err = Some(format!("{}: {e}", path.display()));
                                }
                                outputs.push(None);
                            }
                        }
                    }
                    let mut st = shared.lock().unwrap();
                    *st = BgUiState::Done { outputs, first_err };
                });
            }

            // UI poll @ 10 Hz. Se a janela já foi fechada (cancelled),
            // cortamos o loop imediatamente — widgets podem estar mortos.
            let progress_bar = progress_bar.clone();
            let status = status.clone();
            let apply_btn = apply_btn.clone();
            let cancel_btn = cancel_btn.clone();
            let window = window.clone();
            let originals = files_rc.clone();
            let cancelled_poll = cancelled.clone();
            glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
                if cancelled_poll.load(std::sync::atomic::Ordering::Relaxed) {
                    return glib::ControlFlow::Break;
                }
                let snapshot = shared.lock().unwrap().clone();
                match snapshot {
                    BgUiState::Idle => glib::ControlFlow::Continue,
                    BgUiState::Download { done, total } => {
                        let frac = if total > 0 {
                            (done as f64 / total as f64).clamp(0.0, 1.0)
                        } else {
                            0.0
                        };
                        progress_bar.set_fraction(frac);
                        progress_bar.set_text(Some(&format!(
                            "Baixando modelo (uma vez só): {} / {}",
                            format_size(done),
                            format_size(total),
                        )));
                        glib::ControlFlow::Continue
                    }
                    BgUiState::Processing { idx, total, file } => {
                        let frac = (idx as f64 / total.max(1) as f64).clamp(0.0, 1.0);
                        progress_bar.set_fraction(frac);
                        progress_bar.set_text(Some(&format!(
                            "Processando {}/{} — {}",
                            idx + 1,
                            total,
                            file
                        )));
                        glib::ControlFlow::Continue
                    }
                    BgUiState::Done { outputs, first_err } => {
                        // Checa cancelamento antes de qualquer side-effect:
                        // se o usuário fechou a janela durante a inferência,
                        // não queremos que o Compare apareça sozinho.
                        if cancelled_poll.load(std::sync::atomic::Ordering::Relaxed) {
                            return glib::ControlFlow::Break;
                        }
                        progress_bar.set_fraction(1.0);
                        let ok = outputs.iter().filter(|o| o.is_some()).count();
                        let fail = outputs.len() - ok;
                        if fail == 0 {
                            status.set_text(&format!("Pronto: {ok} imagem(ns) gravada(s)"));
                            progress_bar.set_text(Some("Concluído"));
                            progress_bar.add_css_class("success");
                            let single = originals.len() == 1;
                            if single {
                                if let (Some(orig), Some(Some(out))) =
                                    (originals.first(), outputs.first())
                                {
                                    spawn_compare_viewer(orig, out);
                                }
                                // Single-file: fecha imediato enquanto o
                                // Compare abre — evita janela dupla visível.
                                window.close();
                            } else {
                                // Lote: segura 2 s pra o usuário conferir o
                                // sumário antes de sumir.
                                let w = window.clone();
                                glib::timeout_add_seconds_local_once(2, move || w.close());
                            }
                        } else {
                            let detail = first_err.unwrap_or_default();
                            status.set_text(&format!("{ok} ok, {fail} falha(s). Último: {detail}"));
                            progress_bar.set_text(Some("Com erros"));
                            progress_bar.add_css_class("error");
                            apply_btn.set_sensitive(true);
                            cancel_btn.set_label("Fechar");
                        }
                        glib::ControlFlow::Break
                    }
                }
            });
        });
    }

    window
}

/// Launch the Compare dialog in a separate process so both windows can
/// coexist (each `adw::Application` instance takes over the main loop).
/// Called right after a successful single-file remove-bg so the user
/// sees original vs. result side-by-side.
fn spawn_compare_viewer(original: &Path, result: &Path) {
    let Ok(exe) = std::env::current_exe() else { return };
    let _ =
        std::process::Command::new(exe).arg("--dialog=compare").arg(original).arg(result).spawn();
}

#[cfg(test)]
mod anchor_tests {
    use super::anchor_axis;

    fn approx(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-6, "{a} != {b}");
    }

    #[test]
    fn panned_image_anchors_cursor_point() {
        let (adj, max, _) = anchor_axis(500.0, 500.0, 1.0, 1.2, 2000.0, 1000.0);
        approx(max, 1400.0);
        approx(adj, 700.0);
    }

    #[test]
    fn fit_to_scale_anchors_over_letterbox() {
        let (adj, max, pic) = anchor_axis(600.0, 0.0, 0.5, 1.0, 1000.0, 1000.0);
        approx(pic, 1000.0);
        approx(max, 0.0);
        approx(adj, 0.0);
    }

    #[test]
    fn fit_to_scale_anchors_x_axis_wider_than_viewport() {
        let (adj, max, _) = anchor_axis(800.0, 0.0, 0.5, 1.0, 2000.0, 1000.0);
        approx(max, 1000.0);
        approx(adj, 800.0);
    }

    #[test]
    fn content_smaller_than_viewport_has_zero_scroll() {
        let (adj, max, pic) = anchor_axis(500.0, 0.0, 1.0, 1.5, 400.0, 1000.0);
        approx(pic, 600.0);
        approx(max, 0.0);
        approx(adj, 0.0);
    }

    #[test]
    fn edit_ops_map_to_expected_transforms() {
        use super::{apply_op_to_image, EditOp};
        use image::{DynamicImage, Rgba, RgbaImage};
        // 2×1: pixel esquerdo vermelho, direito verde. Giro à direita → 1×2
        // com vermelho em cima. Cobre também CCW (inverso), 180, flip-H e -V.
        let mut src = RgbaImage::new(2, 1);
        src.put_pixel(0, 0, Rgba([255, 0, 0, 255]));
        src.put_pixel(1, 0, Rgba([0, 255, 0, 255]));
        let img = DynamicImage::ImageRgba8(src);

        let cw = apply_op_to_image(img.clone(), EditOp::RotateCw).to_rgba8();
        assert_eq!(cw.dimensions(), (1, 2));
        assert_eq!(cw.get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert_eq!(cw.get_pixel(0, 1).0, [0, 255, 0, 255]);

        let ccw = apply_op_to_image(img.clone(), EditOp::RotateCcw).to_rgba8();
        assert_eq!(ccw.dimensions(), (1, 2));
        assert_eq!(ccw.get_pixel(0, 0).0, [0, 255, 0, 255]);
        assert_eq!(ccw.get_pixel(0, 1).0, [255, 0, 0, 255]);

        let r180 = apply_op_to_image(img.clone(), EditOp::Rotate180).to_rgba8();
        assert_eq!(r180.dimensions(), (2, 1));
        assert_eq!(r180.get_pixel(0, 0).0, [0, 255, 0, 255]);
        assert_eq!(r180.get_pixel(1, 0).0, [255, 0, 0, 255]);

        let fh = apply_op_to_image(img.clone(), EditOp::FlipH).to_rgba8();
        assert_eq!(fh.get_pixel(0, 0).0, [0, 255, 0, 255]);
        assert_eq!(fh.get_pixel(1, 0).0, [255, 0, 0, 255]);

        let fv = apply_op_to_image(img, EditOp::FlipV).to_rgba8();
        // 1px alto → flip V é no-op visual
        assert_eq!(fv.dimensions(), (2, 1));
        assert_eq!(fv.get_pixel(0, 0).0, [255, 0, 0, 255]);
    }

    #[test]
    fn histogram_counts_channels_and_ignores_alpha() {
        use super::compute_histogram;
        // 3 pixels: puro vermelho, puro verde (com alpha zero), puro azul.
        // Alpha deve ser ignorado — só somamos R/G/B no ponto certo.
        let rgba = vec![
            255, 0, 0, 255, //
            0, 255, 0, 0, //
            0, 0, 255, 255,
        ];
        let h = compute_histogram(&rgba);
        // R: bin 255 += 1 (pixel 0); bin 0 += 2 (pixels 1, 2).
        assert_eq!(h[255], 1);
        assert_eq!(h[0], 2);
        // G: bin 255 += 1 (pixel 1); bin 0 += 2 (pixels 0, 2).
        assert_eq!(h[256 + 255], 1);
        assert_eq!(h[256], 2);
        // B: bin 255 += 1 (pixel 2); bin 0 += 2 (pixels 0, 1).
        assert_eq!(h[512 + 255], 1);
        assert_eq!(h[512], 2);
        // Soma por canal confere com número de pixels.
        let sum = |chan: &[u32]| -> u32 { chan.iter().sum() };
        assert_eq!(sum(&h[0..256]), 3);
        assert_eq!(sum(&h[256..512]), 3);
        assert_eq!(sum(&h[512..768]), 3);
    }

    #[test]
    fn zoom_out_preserves_anchor() {
        let (adj, max, _) = anchor_axis(500.0, 700.0, 1.2, 1.0, 2000.0, 1000.0);
        approx(max, 1000.0);
        approx(adj, 500.0);
    }

    #[test]
    fn default_crop_is_centered_half() {
        use super::default_crop;
        let r = default_crop(100, 60);
        assert_eq!((r.width, r.height), (50, 30));
        assert_eq!((r.x, r.y), (25, 15));
    }

    #[test]
    fn default_crop_handles_degenerate_sizes() {
        // 1×1 → rect is forced to (0,0,1,1), never zero.
        let r = super::default_crop(1, 1);
        assert_eq!((r.x, r.y, r.width, r.height), (0, 0, 1, 1));
    }

    #[test]
    fn contain_fit_letterboxes_wider_widget_than_image() {
        use super::contain_fit;
        // Widget 400×200, image 200×200 → scale 1.0 (height bound),
        // letterbox 100px left + 100px right.
        let (s, ox, oy) = contain_fit(400.0, 200.0, 200.0, 200.0);
        assert!((s - 1.0).abs() < 1e-9);
        assert!((ox - 100.0).abs() < 1e-9);
        assert!(oy.abs() < 1e-9);
    }

    #[test]
    fn contain_fit_scales_down_and_centers() {
        use super::contain_fit;
        // Widget 300×300, image 600×300 → scale 0.5, letterboxed vertically.
        let (s, ox, oy) = contain_fit(300.0, 300.0, 600.0, 300.0);
        assert!((s - 0.5).abs() < 1e-9);
        assert!(ox.abs() < 1e-9);
        assert!((oy - 75.0).abs() < 1e-9);
    }

    #[test]
    fn contain_fit_degenerate_inputs_return_identity() {
        assert_eq!(super::contain_fit(0.0, 100.0, 100.0, 100.0), (1.0, 0.0, 0.0));
        assert_eq!(super::contain_fit(100.0, 100.0, 0.0, 100.0), (1.0, 0.0, 0.0));
    }

    #[test]
    fn decide_drag_mode_picks_resize_when_near_corner() {
        use super::{decide_drag_mode, CropDragMode};
        // Widget 400×400, image 200×200 at scale 2, centered (ox=oy=0).
        // Rect at (50,50,100,100) natural → widget corners at (100,100)..(300,300).
        let mode = decide_drag_mode(
            (98.0, 102.0), // ~2px from top-left handle
            (50.0, 50.0, 100.0, 100.0),
            (2.0, 0.0, 0.0),
            (200, 200),
        );
        match mode {
            CropDragMode::ResizeCorner { anchor_nat, click_nat } => {
                assert_eq!(anchor_nat, (150.0, 150.0)); // opposite = bottom-right
                assert_eq!(click_nat, (50.0, 50.0));
            }
            other => panic!("expected ResizeCorner, got {other:?}"),
        }
    }

    #[test]
    fn decide_drag_mode_picks_move_when_strictly_inside() {
        use super::{decide_drag_mode, CropDragMode};
        let mode = decide_drag_mode(
            (200.0, 200.0), // middle of the rect in widget space (nat 100,100)
            (50.0, 50.0, 100.0, 100.0),
            (2.0, 0.0, 0.0),
            (200, 200),
        );
        match mode {
            CropDragMode::Move { rect0, .. } => {
                assert_eq!(rect0, (50.0, 50.0, 100.0, 100.0));
            }
            other => panic!("expected Move, got {other:?}"),
        }
    }

    #[test]
    fn decide_drag_mode_picks_new_selection_when_outside() {
        use super::{decide_drag_mode, CropDragMode};
        let mode = decide_drag_mode(
            (20.0, 20.0), // far from rect at widget (100,100)-(300,300)
            (50.0, 50.0, 100.0, 100.0),
            (2.0, 0.0, 0.0),
            (200, 200),
        );
        match mode {
            CropDragMode::NewSelection { anchor_nat } => {
                assert_eq!(anchor_nat, (10.0, 10.0));
            }
            other => panic!("expected NewSelection, got {other:?}"),
        }
    }

    #[test]
    fn apply_drag_move_preserves_size_and_clamps() {
        use super::{apply_drag, CropDragMode};
        let mode =
            CropDragMode::Move { rect0: (50.0, 50.0, 100.0, 100.0), click_nat: (100.0, 100.0) };
        // Small move → shift preserved.
        let r = apply_drag(mode, (10.0, -20.0), (200, 200));
        assert_eq!(r, (60.0, 30.0, 100.0, 100.0));
        // Huge move → clamped at image edge, size still preserved.
        let r = apply_drag(mode, (1_000.0, 1_000.0), (200, 200));
        assert_eq!(r, (100.0, 100.0, 100.0, 100.0));
    }

    #[test]
    fn apply_drag_resize_corner_anchors_opposite() {
        use super::{apply_drag, CropDragMode};
        // Anchor is bottom-right (150,150); click started top-left (50,50).
        let mode =
            CropDragMode::ResizeCorner { anchor_nat: (150.0, 150.0), click_nat: (50.0, 50.0) };
        // Move click corner by (+20, +10) in natural pixels.
        let r = apply_drag(mode, (20.0, 10.0), (200, 200));
        // New click = (70, 60); rect becomes (70, 60, 80, 90).
        assert_eq!(r, (70.0, 60.0, 80.0, 90.0));
    }

    #[test]
    fn upscale_batch_doubles_dimensions_lanczos() {
        use super::run_upscale_batch;
        use bigimage_core::OverwritePolicy;
        use image::{Rgb, RgbImage};
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("src.png");
        RgbImage::from_pixel(30, 20, Rgb([10, 20, 30])).save(&path).unwrap();

        let files = vec![path];
        let (ok, skip, fail, err) = run_upscale_batch(&files, 2, None, OverwritePolicy::Replace);
        assert_eq!((ok, skip, fail), (1, 0, 0), "err={err:?}");

        // Output name follows resize's Percent suffix: "_200pct".
        let out = dir.path().join("src_200pct.png");
        assert!(out.exists(), "saída esperada não encontrada: {}", out.display());
        let decoded = image::open(&out).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (60, 40));
    }

    #[test]
    fn upscale_factors_are_restricted_to_2_3_4() {
        // Preserve the contract the CLI advertises — if a future refactor
        // opens the range, this test prompts us to revisit the UI presets.
        let factors: Vec<u8> = super::UPSCALE_FACTORS.iter().map(|(_, f)| *f).collect();
        assert_eq!(factors, vec![2, 3, 4]);
    }

    #[test]
    fn nobg_output_path_is_png_sibling_with_suffix() {
        use super::nobg_output_path;
        use std::path::PathBuf;

        let p = nobg_output_path(&PathBuf::from("/tmp/foo/bar.jpg"));
        assert_eq!(p, PathBuf::from("/tmp/foo/bar_nobg.png"));

        // Sem extensão → ainda vira .png no mesmo diretório.
        let p = nobg_output_path(&PathBuf::from("/tmp/foo/bar"));
        assert_eq!(p, PathBuf::from("/tmp/foo/bar_nobg.png"));

        // Caminho sem diretório → fica no CWD relativo.
        let p = nobg_output_path(&PathBuf::from("solo.png"));
        assert_eq!(p, PathBuf::from("solo_nobg.png"));
    }
}
