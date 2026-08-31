use crate::core::board::Board;
use crate::core::piece::Piece;
use crate::game::Game;
use serde::Deserialize;

#[derive(Deserialize, serde::Serialize)]
pub struct PuzzleJson {
    pub level: u32,
    pub m: u8,
    pub rows: u8,
    pub columns: u8,
    pub board: Vec<Vec<u8>>,
    pub pieces: Vec<Vec<Vec<bool>>>,
    #[serde(default)]
    pub icons: Vec<String>,
}

impl PuzzleJson {
    pub fn load(path: &str) -> Self {
        let data = std::fs::read_to_string(path).expect("failed to read puzzle file");
        serde_json::from_str(&data).expect("failed to parse puzzle JSON")
    }

    pub fn to_game(&self) -> Game {
        let grid: Vec<&[u8]> = self.board.iter().map(|r| r.as_slice()).collect();
        let board = Board::from_grid(&grid, self.m);

        let pieces: Vec<Piece> = self
            .pieces
            .iter()
            .map(|shape| {
                let rows: Vec<&[bool]> = shape.iter().map(|r| r.as_slice()).collect();
                Piece::from_grid(&rows)
            })
            .collect();

        Game::new(board, pieces)
    }
}

const BOARD_JS: &str = include_str!("../web/board.js");

const SOLUTION_CSS: &str = r#"
body { font-family: 'Segoe UI', Arial, sans-serif; background: #1a1a2e; color: #e0e0e0; max-width: 800px; margin: 0 auto; padding: 20px; }
.step-nav { display: flex; align-items: center; justify-content: center; gap: 15px; margin-bottom: 15px; }
.step-nav button { background: #2a2a4a; color: #e0e0e0; border: 1px solid #444; border-radius: 6px; padding: 8px 20px; font-size: 1em; cursor: pointer; }
.step-nav button:hover:not(:disabled) { background: #3a3a5a; }
.step-nav button:disabled { color: #555; cursor: not-allowed; }
.step-label { font-size: 1em; color: #ccc; min-width: 180px; text-align: center; }
.board { display: inline-grid; gap: 0; padding: 0; margin: 0 auto; display: grid; justify-content: center; }
.cell { width: 50px; height: 50px; position: relative; line-height: 0; }
.cell img { width: 50px; height: 50px; display: block; }
.cell.click-here { outline: 4px solid #2ecc40; outline-offset: -4px; }
.solved { color: #2ecc71; font-size: 24px; text-align: center; font-weight: bold; padding: 20px; }
"#;

/// Generate an HTML guide with all puzzle data and JavaScript embedded.
pub fn generate_html_guide(
    puzzle: &PuzzleJson,
    solution: &[(usize, usize)],
    assets_dir: &str,
) -> String {
    let placements_json =
        serde_json::to_string(&solution.iter().map(|&(r, c)| [r, c]).collect::<Vec<_>>()).unwrap();
    let puzzle_json = serde_json::to_string(puzzle).unwrap();

    format!(
        r#"<!DOCTYPE html>
<html>
<head>
<meta charset="UTF-8">
<title>Shapeshifter Level {level} Solution</title>
<style>{css}</style>
</head>
<body>
<div id="solution" style="text-align:center"></div>
<script>
{board_js}
</script>
<script>
var puzzle = {puzzle_json};
var placements = {placements_json};
var assetsDir = "{assets_dir}";
boardShowSolution(document.getElementById('solution'), puzzle, placements, assetsDir);
</script>
</body>
</html>"#,
        level = puzzle.level,
        css = SOLUTION_CSS,
        board_js = BOARD_JS,
        puzzle_json = puzzle_json,
        placements_json = placements_json,
        assets_dir = assets_dir,
    )
}
