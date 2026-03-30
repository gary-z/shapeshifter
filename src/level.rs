use serde::Deserialize;

/// A level specification loaded from data/levels.json.
#[derive(Debug, Clone, Deserialize)]
pub struct LevelSpec {
    pub level: u32,
    /// M: the modulus (called "shifts" in the game).
    pub shifts: u8,
    pub rows: u8,
    pub columns: u8,
    /// Number of pieces (called "shapes" in the game).
    pub shapes: u8,
}

/// Load all level specs from the embedded JSON.
pub fn load_levels() -> Vec<LevelSpec> {
    let data = include_str!("../data/levels.json");
    serde_json::from_str(data).expect("failed to parse levels.json")
}

/// Get the spec for a specific level (1-indexed).
pub fn get_level(level: u32) -> Option<LevelSpec> {
    load_levels().into_iter().find(|l| l.level == level)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_all_levels() {
        let levels = load_levels();
        assert_eq!(levels.len(), 100);
        assert_eq!(levels[0].level, 1);
        assert_eq!(levels[99].level, 100);
    }

    #[test]
    fn test_level_1() {
        let spec = get_level(1).unwrap();
        assert_eq!(spec.shifts, 2);
        assert_eq!(spec.rows, 3);
        assert_eq!(spec.columns, 3);
        assert_eq!(spec.shapes, 2);
    }

    #[test]
    fn test_level_100() {
        let spec = get_level(100).unwrap();
        assert_eq!(spec.shifts, 5);
        assert_eq!(spec.rows, 14);
        assert_eq!(spec.columns, 14);
        assert_eq!(spec.shapes, 36);
    }

    #[test]
    fn test_invalid_level() {
        assert!(get_level(0).is_none());
        assert!(get_level(101).is_none());
    }
}
