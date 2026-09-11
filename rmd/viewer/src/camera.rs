use render::Camera;

pub struct Controller {
    pub camera: Camera,
    dragging: bool,
    cursor: (f32, f32),
}

impl Default for Controller {
    fn default() -> Self { Self::new() }
}

impl Controller {
    pub fn new() -> Self {
        Self {
            camera: Camera::default(),
            dragging: false,
            cursor: (0.0, 0.0),
        }
    }

    pub fn frame_map(&mut self, width_px: f32, height_px: f32) {
        self.camera.x = width_px / 2.0;
        self.camera.y = height_px / 2.0;

        let (viewport_w, viewport_h) = (
            self.camera.viewport_width.max(1) as f32,
            self.camera.viewport_height.max(1) as f32,
        );
        let fit = (viewport_w / width_px.max(1.0)).min(viewport_h / height_px.max(1.0));

        self.camera.zoom = (fit * 0.95).clamp(0.05, 64.0);
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.camera.viewport_width = width;
        self.camera.viewport_height = height;
    }

    pub fn set_dragging(&mut self, dragging: bool) { self.dragging = dragging; }

    pub fn cursor_moved(&mut self, x: f32, y: f32) {
        let (dx, dy) = (x - self.cursor.0, y - self.cursor.1);
        self.cursor = (x, y);

        if self.dragging {
            let zoom = self.camera.zoom.max(f32::EPSILON);
            self.camera.x -= dx / zoom;
            self.camera.y += dy / zoom;
        }
    }

    pub fn zoom_by(&mut self, steps: f32) {
        let before = self.screen_to_map(self.cursor);
        self.camera.zoom = (self.camera.zoom * 1.2_f32.powf(steps)).clamp(0.05, 64.0);
        let after = self.screen_to_map(self.cursor);

        self.camera.x += before.0 - after.0;
        self.camera.y += before.1 - after.1;
    }

    fn screen_to_map(&self, (x, y): (f32, f32)) -> (f32, f32) {
        let zoom = self.camera.zoom.max(f32::EPSILON);
        let half_w = self.camera.viewport_width as f32 / 2.0;
        let half_h = self.camera.viewport_height as f32 / 2.0;

        (self.camera.x + (x - half_w) / zoom, self.camera.y - (y - half_h) / zoom)
    }
}
