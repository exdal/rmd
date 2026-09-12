use dmm::{Coord, Size};
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

    pub fn center_on_tile(&mut self, coord: Coord, tile_size: u32) {
        let tile_size = tile_size.max(1) as f32;
        self.camera.x = (coord.x.saturating_sub(1) as f32 + 0.5) * tile_size;
        self.camera.y = (coord.y.saturating_sub(1) as f32 + 0.5) * tile_size;
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

    pub fn screen_to_map(&self, point: [f32; 2]) -> [f32; 2] {
        let zoom = self.camera.zoom.max(f32::EPSILON);
        let half_w = self.camera.viewport_width as f32 / 2.0;
        let half_h = self.camera.viewport_height as f32 / 2.0;

        [
            self.camera.x + (point[0] - half_w) / zoom,
            self.camera.y - (point[1] - half_h) / zoom,
        ]
    }

    pub fn screen_to_tile(&self, point: [f32; 2], size: Size, tile_size: u32, z: u32) -> Option<Coord> {
        let map = self.screen_to_map(point);
        let tile_size = tile_size.max(1) as f32;
        let width = size.x as f32 * tile_size;
        let height = size.y as f32 * tile_size;

        if !map[0].is_finite()
            || !map[1].is_finite()
            || map[0] < 0.0
            || map[1] < 0.0
            || map[0] >= width
            || map[1] >= height
        {
            return None;
        }

        Some(Coord::new(
            (map[0] / tile_size).floor() as u32 + 1,
            (map[1] / tile_size).floor() as u32 + 1,
            z,
        ))
    }

    pub fn map_to_screen(&self, point: [f32; 2]) -> [f32; 2] {
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
    use dmm::{Coord, Size};

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

    #[test]
    fn screen_position_resolves_one_based_map_tile() {
        let mut controller = Controller::new();
        controller.resize(64, 64);
        controller.camera.x = 32.0;
        controller.camera.y = 32.0;
        let size = Size { x: 2, y: 2, z: 1 };

        assert_eq!(
            controller.screen_to_tile([16.0, 48.0], size, 32, 1),
            Some(Coord::new(1, 1, 1))
        );
        assert_eq!(
            controller.screen_to_tile([48.0, 16.0], size, 32, 1),
            Some(Coord::new(2, 2, 1))
        );
    }

    #[test]
    fn screen_position_outside_map_has_no_tile() {
        let mut controller = Controller::new();
        controller.resize(64, 64);
        controller.camera.x = 32.0;
        controller.camera.y = 32.0;
        let size = Size { x: 2, y: 2, z: 1 };

        assert_eq!(controller.screen_to_tile([64.0, 32.0], size, 32, 1), None);
        assert_eq!(controller.screen_to_tile([32.0, 0.0], size, 32, 1), None);
    }

    #[test]
    fn centering_on_a_tile_preserves_zoom() {
        let mut controller = Controller::new();
        controller.camera.zoom = 2.5;

        controller.center_on_tile(Coord::new(3, 4, 2), 32);

        assert_eq!(controller.camera.x, 80.0);
        assert_eq!(controller.camera.y, 112.0);
        assert_eq!(controller.camera.zoom, 2.5);
    }
}
