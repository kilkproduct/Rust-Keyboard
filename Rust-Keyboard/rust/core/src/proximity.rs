//! Touch geometry: which characters a touch point plausibly means, and how far
//! away it is from a key.
//!
//! The Java side computes, at keyboard-layout time, a grid over the keyboard
//! where every cell lists the (up to 16) characters that are within 1.2 key
//! widths of that cell's centre.  That table is handed to us once per layout and
//! is kept in one flat `Vec<i32>` -- no hashing, no per-point allocation.

use crate::MAX_PROXIMITY_CHARS_SIZE;

pub const NOT_A_CODE: i32 = -1;
pub const NOT_AN_INDEX: i32 = -1;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProximityType {
    /// The character under the finger.
    MatchChar,
    /// A character on a neighbouring key.
    ProximityChar,
    /// A reachable character that is not adjacent.
    UnrelatedChar,
}

pub struct ProximityInfo {
    grid_width: i32,
    grid_height: i32,
    cell_width: i32,
    cell_height: i32,
    keyboard_width: i32,
    keyboard_height: i32,
    most_common_key_width: i32,
    most_common_key_height: i32,
    key_count: usize,
    /// `grid_width * grid_height * MAX_PROXIMITY_CHARS_SIZE` code points.
    proximity_chars: Vec<i32>,
    key_x: Vec<i32>,
    key_y: Vec<i32>,
    key_width: Vec<i32>,
    key_height: Vec<i32>,
    key_code_point: Vec<i32>,
    sweet_x: Vec<f32>,
    sweet_y: Vec<f32>,
    sweet_radius: Vec<f32>,
    /// `(lowered code point, key index)` sorted by code point.
    lower_code_point_to_key: Vec<(i32, i32)>,
}

impl ProximityInfo {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        keyboard_width: i32,
        keyboard_height: i32,
        grid_width: i32,
        grid_height: i32,
        most_common_key_width: i32,
        most_common_key_height: i32,
        proximity_chars: Vec<i32>,
        key_count: usize,
        key_x: Vec<i32>,
        key_y: Vec<i32>,
        key_width: Vec<i32>,
        key_height: Vec<i32>,
        key_code_point: Vec<i32>,
        sweet_x: Vec<f32>,
        sweet_y: Vec<f32>,
        sweet_radius: Vec<f32>,
    ) -> ProximityInfo {
        let safe_grid_width = grid_width.max(1);
        let safe_grid_height = grid_height.max(1);
        let cell_width = ((keyboard_width + safe_grid_width - 1) / safe_grid_width).max(1);
        let cell_height = ((keyboard_height + safe_grid_height - 1) / safe_grid_height).max(1);

        let mut lower_code_point_to_key: Vec<(i32, i32)> = Vec::with_capacity(key_count);
        for index in 0..key_count {
            let code_point = *key_code_point.get(index).unwrap_or(&NOT_A_CODE);
            if code_point == NOT_A_CODE {
                continue;
            }
            lower_code_point_to_key.push((
                crate::charutils::to_lower(code_point),
                index as i32,
            ));
        }
        lower_code_point_to_key.sort_by(|a, b| a.0.cmp(&b.0));
        lower_code_point_to_key.dedup_by(|a, b| a.0 == b.0);

        ProximityInfo {
            grid_width: safe_grid_width,
            grid_height: safe_grid_height,
            cell_width,
            cell_height,
            keyboard_width,
            keyboard_height,
            most_common_key_width: most_common_key_width.max(1),
            most_common_key_height: most_common_key_height.max(1),
            key_count,
            proximity_chars,
            key_x,
            key_y,
            key_width,
            key_height,
            key_code_point,
            sweet_x,
            sweet_y,
            sweet_radius,
            lower_code_point_to_key,
        }
    }

    pub fn key_count(&self) -> usize {
        self.key_count
    }

    pub fn most_common_key_width(&self) -> i32 {
        self.most_common_key_width
    }

    pub fn key_code_point(&self, key_index: usize) -> i32 {
        *self.key_code_point.get(key_index).unwrap_or(&NOT_A_CODE)
    }

    /// Key index of a code point, or [`NOT_AN_INDEX`].
    pub fn key_index_of(&self, code_point: i32) -> i32 {
        let lowered = crate::charutils::to_lower(code_point);
        match self
            .lower_code_point_to_key
            .binary_search_by(|probe| probe.0.cmp(&lowered))
        {
            Ok(index) => self.lower_code_point_to_key[index].1,
            Err(_) => NOT_AN_INDEX,
        }
    }

    pub fn is_code_point_on_keyboard(&self, code_point: i32) -> bool {
        self.key_index_of(code_point) != NOT_AN_INDEX
    }

    fn cell_index(&self, x: i32, y: i32) -> usize {
        let column = (x / self.cell_width).clamp(0, self.grid_width - 1);
        let row = (y / self.cell_height).clamp(0, self.grid_height - 1);
        (row * self.grid_width + column) as usize
    }

    /// Copies the candidate characters of the cell that contains `(x, y)`.
    pub fn proximity_chars_at(
        &self,
        x: i32,
        y: i32,
        out: &mut [i32; MAX_PROXIMITY_CHARS_SIZE],
    ) -> usize {
        let cell = self.cell_index(x, y);
        let base = cell * MAX_PROXIMITY_CHARS_SIZE;
        let mut count = 0usize;
        while count < MAX_PROXIMITY_CHARS_SIZE {
            match self.proximity_chars.get(base + count) {
                Some(code_point) if *code_point != NOT_A_CODE && *code_point != 0 => {
                    out[count] = *code_point;
                    count += 1;
                }
                _ => break,
            }
        }
        count
    }

    /// Classifies `code_point` relative to the touch point `(x, y)`.
    pub fn proximity_type(&self, primary: i32, code_point: i32, x: i32, y: i32) -> ProximityType {
        if crate::charutils::to_lower(primary) == crate::charutils::to_lower(code_point) {
            return ProximityType::MatchChar;
        }
        let cell = self.cell_index(x, y);
        let base = cell * MAX_PROXIMITY_CHARS_SIZE;
        let lowered = crate::charutils::to_lower(code_point);
        for offset in 0..MAX_PROXIMITY_CHARS_SIZE {
            match self.proximity_chars.get(base + offset) {
                Some(candidate) if *candidate != NOT_A_CODE && *candidate != 0 => {
                    if crate::charutils::to_lower(*candidate) == lowered {
                        return ProximityType::ProximityChar;
                    }
                }
                _ => break,
            }
        }
        ProximityType::UnrelatedChar
    }

    pub fn has_sweet_spot(&self, key_index: usize) -> bool {
        match self.sweet_radius.get(key_index) {
            Some(radius) => *radius > 0.0f32,
            None => false,
        }
    }

    fn key_center_x(&self, key_index: usize) -> f32 {
        if self.has_sweet_spot(key_index) {
            if let Some(value) = self.sweet_x.get(key_index) {
                return *value;
            }
        }
        let x = *self.key_x.get(key_index).unwrap_or(&0) as f32;
        let width = *self.key_width.get(key_index).unwrap_or(&0) as f32;
        x + width / 2.0f32
    }

    fn key_center_y(&self, key_index: usize) -> f32 {
        if self.has_sweet_spot(key_index) {
            if let Some(value) = self.sweet_y.get(key_index) {
                return *value;
            }
        }
        let y = *self.key_y.get(key_index).unwrap_or(&0) as f32;
        let height = *self.key_height.get(key_index).unwrap_or(&0) as f32;
        y + height / 2.0f32
    }

    /// Squared distance from `(x, y)` to the centre of `key_index`, normalised
    /// so that a whole key width is `1.0`.
    pub fn normalized_squared_distance(&self, key_index: i32, x: i32, y: i32) -> f32 {
        if key_index < 0 {
            return 1.0f32;
        }
        let index = key_index as usize;
        if index >= self.key_count {
            return 1.0f32;
        }
        let dx = self.key_center_x(index) - x as f32;
        let dy = self.key_center_y(index) - y as f32;
        let width = self.most_common_key_width as f32;
        let distance = (dx * dx + dy * dy) / (width * width);
        // Clamp: anything beyond a key and a half away is "as far as it gets".
        if distance > 1.0f32 {
            1.0f32
        } else {
            distance
        }
    }

    /// Distance used when a code point is not on the keyboard at all.
    pub fn distance_for_code_point(&self, code_point: i32, x: i32, y: i32) -> f32 {
        let key_index = self.key_index_of(code_point);
        if key_index == NOT_AN_INDEX {
            // A character that cannot be typed (e.g. an apostrophe on a narrow
            // layout): treat it as a neighbouring key.
            0.5f32
        } else {
            self.normalized_squared_distance(key_index, x, y)
        }
    }

    pub fn keyboard_width(&self) -> i32 {
        self.keyboard_width
    }

    pub fn keyboard_height(&self) -> i32 {
        self.keyboard_height
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn materialize() -> ProximityInfo {
        // Two keys side by side, a 2x1 grid, both chars in each cell.
        let key_x = vec![0, 100];
        let key_y = vec![0, 0];
        let key_w = vec![100, 100];
        let key_h = vec![100, 100];
        let key_cp = vec![0x61, 0x62]; // a, b
        let mut proximity = vec![NOT_A_CODE; 2 * MAX_PROXIMITY_CHARS_SIZE];
        proximity[0] = 0x61;
        proximity[1] = 0x62;
        proximity[MAX_PROXIMITY_CHARS_SIZE] = 0x61;
        proximity[MAX_PROXIMITY_CHARS_SIZE + 1] = 0x62;
        ProximityInfo::new(
            200,
            100,
            2,
            1,
            100,
            100,
            proximity,
            2,
            key_x,
            key_y,
            key_w,
            key_h,
            key_cp,
            vec![0.0, 0.0],
            vec![0.0, 0.0],
            vec![0.0, 0.0],
        )
    }

    #[test]
    fn finds_key_index() {
        let info = materialize();
        assert_eq!(info.key_index_of(0x61), 0);
        assert_eq!(info.key_index_of(0x62), 1);
        assert_eq!(info.key_index_of(0x7A), NOT_AN_INDEX);
        assert_eq!(info.key_index_of(0x41), 0); // case insensitive
    }

    #[test]
    fn classifies_proximity() {
        let info = materialize();
        assert_eq!(
            info.proximity_type(0x61, 0x61, 10, 50),
            ProximityType::MatchChar
        );
        assert_eq!(
            info.proximity_type(0x61, 0x62, 10, 50),
            ProximityType::ProximityChar
        );
        assert_eq!(
            info.proximity_type(0x61, 0x7A, 10, 50),
            ProximityType::UnrelatedChar
        );
    }

    #[test]
    fn distance_is_small_under_the_key() {
        let info = materialize();
        let near = info.normalized_squared_distance(0, 50, 50);
        let far = info.normalized_squared_distance(0, 190, 50);
        assert!(near < 0.01);
        assert!(far > near);
    }

    #[test]
    fn reads_cell_characters() {
        let info = materialize();
        let mut out = [0i32; MAX_PROXIMITY_CHARS_SIZE];
        let count = info.proximity_chars_at(10, 10, &mut out);
        assert_eq!(count, 2);
        assert_eq!(out[0], 0x61);
        assert_eq!(out[1], 0x62);
    }
}
