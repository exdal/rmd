use render::Camera;

pub struct Controller {
    pub camera: Camera,
}

impl Default for Controller {
    fn default() -> Self { Self::new() }
}

impl Controller {
    pub fn new() -> Self {
        Self {
            camera: Camera::default(),
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

    pub fn pan_by(&mut self, delta: [f32; 2]) {
        let zoom = self.camera.zoom.max(f32::EPSILON);
        self.camera.x -= delta[0] / zoom;
        self.camera.y += delta[1] / zoom;
    }

    pub fn zoom_by(&mut self, steps: f32, cursor: [f32; 2]) {
        let before = self.screen_to_map(cursor);
        self.camera.zoom = (self.camera.zoom * 1.2_f32.powf(steps)).clamp(0.05, 64.0);
        let after = self.screen_to_map(cursor);

        self.camera.x += before[0] - after[0];
        self.camera.y += before[1] - after[1];
    }

    fn screen_to_map(&self, point: [f32; 2]) -> [f32; 2] {
        let zoom = self.camera.zoom.max(f32::EPSILON);
        let half_w = self.camera.viewport_width as f32 / 2.0;
        let half_h = self.camera.viewport_height as f32 / 2.0;

        [
            self.camera.x + (point[0] - half_w) / zoom,
            self.camera.y - (point[1] - half_h) / zoom,
        ]
    }

    #[cfg(test)]
    fn map_to_screen(&self, point: [f32; 2]) -> [f32; 2] {
        let half_w = self.camera.viewport_width as f32 / 2.0;
        let half_h = self.camera.viewport_height as f32 / 2.0;

        [
            (point[0] - self.camera.x) * self.camera.zoom + half_w,
            (self.camera.y - point[1]) * self.camera.zoom + half_h,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::Controller;

    #[test]
    fn panel_coordinates_round_trip() {
        let mut controller = Controller::new();
        controller.resize(800, 600);
        controller.camera.x = 120.0;
        controller.camera.y = 80.0;
        controller.camera.zoom = 2.5;
        let point = [140.0, 45.0];

        let screen = controller.map_to_screen(point);

        assert_eq!(controller.screen_to_map(screen), point);
    }

    #[test]
    fn zoom_stays_anchored_at_the_cursor() {
        let mut controller = Controller::new();
        controller.resize(900, 500);
        controller.camera.x = 300.0;
        controller.camera.y = 200.0;
        controller.camera.zoom = 1.5;
        let cursor = [120.0, 310.0];
        let before = controller.screen_to_map(cursor);

        controller.zoom_by(3.0, cursor);

        let after = controller.screen_to_map(cursor);
        assert!((before[0] - after[0]).abs() < 0.0001);
        assert!((before[1] - after[1]).abs() < 0.0001);
    }
}
