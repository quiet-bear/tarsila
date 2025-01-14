use crate::bg::Background;
use crate::graphics::DrawContext;
use crate::gui::{Gui, GuiSyncParams};
use crate::keyboard::KeyboardManager;
use crate::mouse::MouseManager;
use crate::wrapped_image::WrappedImage;
use crate::{graphics, Timer};
use lapix::primitives::*;
use lapix::{Canvas, CanvasEffect, Event, Layer, Selection, State, Tool};
use macroquad::prelude::Color as MqColor;
use macroquad::prelude::{FilterMode, Texture2D};
use std::default::Default;
use std::time::SystemTime;

pub const WINDOW_W: i32 = 1000;
pub const WINDOW_H: i32 = 600;
pub const CANVAS_W: u16 = 64;
pub const CANVAS_H: u16 = 64;
const CANVAS_SCALE: f32 = 8.;
const LEFT_TOOLBAR_W: u16 = 300;
const CAMERA_SPEED: f32 = 12.;
const BG_COLOR: MqColor = MqColor::new(0.5, 0.5, 0.5, 1.);
const GUI_REST_MS: u64 = 100;
const FPS_INTERVAL: usize = 15;
const DEFAULT_ZOOM_LEVEL: f32 = 8.;

// Center on the space after the toolbar
const CANVAS_X: f32 = LEFT_TOOLBAR_W as f32 + ((WINDOW_W as u16 - LEFT_TOOLBAR_W) / 2) as f32
    - (CANVAS_W as f32 * CANVAS_SCALE / 2.);
const CANVAS_Y: f32 = (WINDOW_H / 2) as f32 - (CANVAS_H as f32 * CANVAS_SCALE / 2.);

#[derive(Debug, Clone)]
pub enum Effect {
    Event(Event),
    UiEvent(UiEvent),
}

impl From<Event> for Effect {
    fn from(val: Event) -> Self {
        Self::Event(val)
    }
}

impl From<UiEvent> for Effect {
    fn from(val: UiEvent) -> Self {
        Self::UiEvent(val)
    }
}

// TODO remove this
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum UiEvent {
    ZoomIn,
    ZoomOut,
    ResetZoom,
    MoveCamera(Direction),
    MouseOverGui,
    Paste,
    Exit,
    NewProject,
    GuiInteraction,
    SetZoom100,
}

impl UiEvent {
    pub fn is_gui_interaction(&self) -> bool {
        matches!(self, Self::MouseOverGui | Self::GuiInteraction)
    }
}

impl<'a> From<&'a UiState> for GuiSyncParams {
    fn from(state: &'a UiState) -> Self {
        let n_layers = state.inner.layers().count();
        let (x, y) = macroquad::prelude::mouse_position();
        let (x, y) = state.screen_to_canvas(x, y);
        let p = (x, y).into();
        let in_canvas = state.canvas().is_in_bounds(p);
        let visible_pixel = if in_canvas {
            Some(state.visible_pixel(p))
        } else {
            None
        };

        Self {
            main_color: state.inner.main_color().into(),
            num_layers: n_layers,
            active_layer: state.inner.layers().active_index(),
            layers_vis: (0..n_layers)
                .map(|i| state.inner.layers().get(i).visible())
                .collect(),
            layers_alpha: (0..n_layers)
                .map(|i| state.inner.layers().get(i).opacity())
                .collect(),
            palette: state.inner.palette().iter().map(|c| (*c).into()).collect(),
            mouse_canvas: (x, y).into(),
            is_on_canvas: in_canvas,
            selected_tool: state.selected_tool(),
            visible_pixel_on_mouse: visible_pixel,
            canvas_size: state.canvas().size(),
            spritesheet: state.inner.spritesheet(),
            zoom: state.zoom,
            fps: state.fps,
        }
    }
}

pub struct UiState {
    inner: State<WrappedImage>,
    gui: Gui,
    camera: Position<f32>,
    canvas_pos: Position<f32>,
    zoom: f32,
    layer_textures: Vec<Texture2D>,
    keyboard: KeyboardManager,
    mouse: MouseManager,
    mouse_over_gui: bool,
    gui_interaction_rest: Timer,
    free_image_tex: Option<Texture2D>,
    must_exit: bool,
    t0: SystemTime,
    fps: f32,
    bg: Background,
}

impl Default for UiState {
    fn default() -> Self {
        let state = State::<WrappedImage>::new((CANVAS_W as i32, CANVAS_H as i32).into());
        let drawing = Texture2D::from_image(&state.canvas().inner().0);
        drawing.set_filter(FilterMode::Nearest);

        Self {
            inner: state,
            gui: Gui::new(),
            camera: Position::ZERO_F32,
            canvas_pos: (CANVAS_X, CANVAS_Y).into(),
            zoom: DEFAULT_ZOOM_LEVEL,
            layer_textures: vec![drawing],
            keyboard: KeyboardManager::new(),
            mouse: MouseManager::new(),
            mouse_over_gui: false,
            gui_interaction_rest: Timer::new(),
            free_image_tex: None,
            must_exit: false,
            t0: SystemTime::now(),
            fps: 60.,
            bg: Background::new(),
        }
    }
}

impl UiState {
    pub fn must_exit(&self) -> bool {
        self.must_exit
    }

    /*pub fn drawing_mut(&mut self) -> &mut Texture2D {
        &mut self.layer_textures[self.inner.layers().active_index()]
    }*/

    pub fn drawing(&self) -> &Texture2D {
        &self.layer_textures[self.inner.layers().active_index()]
    }

    pub fn update(&mut self, frame: usize) {
        if frame % FPS_INTERVAL == (FPS_INTERVAL - 1) {
            let elapsed_ms = self.t0.elapsed().unwrap().as_millis();
            self.fps = FPS_INTERVAL as f32 / (elapsed_ms as f32 / 1000.);
            self.t0 = SystemTime::now();
        }

        self.mouse_over_gui = false;

        self.gui.sync((&*self).into());
        let fx = self.gui.update();
        self.process_fx(fx);

        self.sync_mouse();
        let fx = self.mouse.update();
        self.process_fx(fx);

        let fx = self.keyboard.update();
        self.process_fx(fx);
    }

    fn process_fx(&mut self, fx: Vec<Effect>) {
        for effect in fx {
            match effect {
                Effect::UiEvent(event) => self.process_event(event),
                Effect::Event(event) => {
                    self.execute(event);
                }
            }
        }
    }

    fn is_canvas_blocked(&self) -> bool {
        self.mouse_over_gui || !self.gui_interaction_rest.expired()
    }

    fn draw_ctx(&self) -> DrawContext {
        DrawContext {
            spritesheet: self.inner.spritesheet(),
            scale: self.zoom(),
            canvas_pos: self.canvas_pos(),
            camera: self.camera(),
            canvas_size: (self.canvas().width() as f32, self.canvas().height() as f32).into(),
            selection: self.inner.selection(),
        }
    }

    pub fn draw(&mut self) {
        macroquad::prelude::clear_background(BG_COLOR);

        let ctx = self.draw_ctx();

        self.bg.draw(ctx);
        graphics::draw_canvas(&*self);
        graphics::draw_spritesheet_boundaries(ctx);

        let (x, y) = macroquad::prelude::mouse_position();
        let mouse_canvas = self.screen_to_canvas(x, y).into();

        // TODO should be in update method
        self.inner.update_free_image(mouse_canvas);

        if self.inner.selection().is_some() {
            graphics::draw_selection(ctx, self.inner.free_image());
        }

        // TODO: most of this logic should be in some update method, not a draw one
        if let Some(img) = self.inner.free_image() {
            let tex = Texture2D::from_image(&img.texture.0);
            tex.set_filter(FilterMode::Nearest);
            self.free_image_tex = Some(tex);

            graphics::draw_free_image(
                ctx,
                img,
                self.inner.layers().active().opacity(),
                self.free_image_tex.unwrap(),
            );
        } else {
            self.free_image_tex = None;
        }

        egui_macroquad::draw();
        self.gui.draw_preview(self);
        self.mouse.draw(self.selected_tool());
    }

    pub fn sync_mouse(&mut self) {
        let (x, y) = macroquad::prelude::mouse_position();
        let (x, y) = self.screen_to_canvas(x, y);
        let p = (x, y).into();
        let in_canvas = self.canvas().is_in_bounds(p);
        let visible_pixel = if in_canvas {
            Some(self.visible_pixel(p))
        } else {
            None
        };

        self.mouse.sync(
            (x, y).into(),
            in_canvas,
            self.is_mouse_on_selection(),
            self.selected_tool(),
            visible_pixel,
            self.is_canvas_blocked(),
        );
    }

    pub fn execute(&mut self, event: Event) {
        let effect = self.inner.execute(event);

        match effect {
            // TODO: Texture2D is copy, so we don't need `drawing_mut` here, but
            // it would be better.
            CanvasEffect::Update => {
                self.drawing().update(&self.canvas().inner().0);
            }
            CanvasEffect::New | CanvasEffect::Layer => {
                self.sync_layer_textures();
            }
            CanvasEffect::None => (),
        };
    }

    pub fn sync_layer_textures(&mut self) {
        for layer in 0..self.inner.layers().count() {
            self.sync_layer_texture(layer);
        }
    }

    pub fn sync_layer_texture(&mut self, index: usize) {
        let layer_img = &self.inner.layers().canvas_at(index).inner().0;
        let texture = Texture2D::from_image(layer_img);
        texture.set_filter(FilterMode::Nearest);

        match self.layer_textures.get_mut(index) {
            Some(tex) => *tex = texture,
            None => self.layer_textures.push(texture),
        }
    }

    pub fn process_event(&mut self, event: UiEvent) {
        if event.is_gui_interaction() {
            self.gui_interaction_rest.start(GUI_REST_MS);
        }
        match event {
            UiEvent::ZoomIn => self.zoom_in(),
            UiEvent::ZoomOut => self.zoom_out(),
            UiEvent::ResetZoom => self.reset_zoom(),
            UiEvent::SetZoom100 => self.zoom = 1.,
            UiEvent::MoveCamera(dir) => self.move_camera(dir),
            UiEvent::MouseOverGui => self.mouse_over_gui = true,
            UiEvent::GuiInteraction => (),
            UiEvent::Paste => {
                let (x, y) = macroquad::prelude::mouse_position();
                let (x, y) = self.screen_to_canvas(x, y);
                self.execute(Event::Paste((x, y).into()));
            }
            UiEvent::Exit => self.must_exit = true,
            UiEvent::NewProject => *self = UiState::default(),
        }
    }

    pub fn visible_pixel(&self, p: Point<i32>) -> [u8; 4] {
        self.inner.layers().visible_pixel(p).into()
    }

    pub fn camera(&self) -> Position<f32> {
        self.camera
    }

    pub fn canvas(&self) -> &Canvas<WrappedImage> {
        self.inner.canvas()
    }

    pub fn canvas_pos(&self) -> Position<f32> {
        self.canvas_pos
    }

    pub fn canvas_actual_size(&self) -> Size<f32> {
        (
            self.inner.canvas().width() as f32 * self.zoom,
            self.inner.canvas().height() as f32 * self.zoom,
        )
            .into()
    }

    pub fn selected_tool(&self) -> Tool {
        self.inner.selected_tool()
    }

    pub fn zoom(&self) -> f32 {
        self.zoom
    }

    pub fn layer(&self, index: usize) -> &Layer<WrappedImage> {
        self.inner.layers().get(index)
    }

    pub fn num_layers(&self) -> usize {
        self.inner.layers().count()
    }

    pub fn layer_tex(&self, index: usize) -> Texture2D {
        self.layer_textures[index]
    }

    pub fn zoom_in(&mut self) {
        self.zoom *= 2.;
        self.camera.x *= 2.;
        self.camera.y *= 2.;
    }

    pub fn zoom_out(&mut self) {
        self.zoom /= 2.;
        self.camera.x /= 2.;
        self.camera.y /= 2.;
    }

    pub fn reset_zoom(&mut self) {
        self.zoom = DEFAULT_ZOOM_LEVEL;
    }

    pub fn move_camera(&mut self, direction: Direction) {
        let speed = CAMERA_SPEED;

        if !self.is_camera_off(direction) {
            match direction {
                Direction::Up => self.camera.y -= speed,
                Direction::Down => self.camera.y += speed,
                Direction::Left => self.camera.x -= speed,
                Direction::Right => self.camera.x += speed,
            }
        }
    }

    fn is_camera_off(&self, direction: Direction) -> bool {
        let buffer = 20.;
        let canvas_size = self.canvas_actual_size();
        let canvas_pos = self.canvas_pos;
        let camera = self.camera;
        let win_w = WINDOW_W as f32;
        let win_h = WINDOW_H as f32;

        match direction {
            Direction::Up => canvas_pos.y - camera.y > win_h - buffer,
            Direction::Down => camera.y > canvas_pos.y + canvas_size.y - buffer,
            Direction::Left => canvas_pos.x - camera.x > win_w - buffer,
            Direction::Right => camera.x > canvas_pos.x + canvas_size.x - buffer,
        }
    }

    pub fn screen_to_canvas(&self, x: f32, y: f32) -> (i32, i32) {
        let canvas_x = self.canvas_pos().x - self.camera().x;
        let canvas_y = self.canvas_pos().y - self.camera().y;
        let scale = self.zoom();

        (
            ((x - canvas_x) / scale) as i32,
            ((y - canvas_y) / scale) as i32,
        )
    }

    pub fn is_mouse_on_selection(&self) -> bool {
        let (x, y) = macroquad::prelude::mouse_position();
        let (x, y) = self.screen_to_canvas(x, y);

        let rect = match self.inner.selection() {
            Some(Selection::FreeImage) => self.inner.free_image().unwrap().rect,
            Some(Selection::Canvas(rect)) => rect,
            _ => return false,
        };

        rect.contains(x, y)
    }
}
