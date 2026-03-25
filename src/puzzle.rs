use serde::Deserialize;
use crate::core::board::Board;
use crate::game::Game;
use crate::core::piece::Piece;

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

        let pieces: Vec<Piece> = self.pieces.iter().map(|shape| {
            let rows: Vec<&[bool]> = shape.iter().map(|r| r.as_slice()).collect();
            Piece::from_grid(&rows)
        }).collect();

        Game::new(board, pieces)
    }

    /// Get the image path for a cell value, relative to the data directory.
    pub fn icon_src(&self, val: u8, assets_dir: &str) -> String {
        if !self.icons.is_empty() && (val as usize) < self.icons.len() {
            format!("{}/{}_0.gif", assets_dir, self.icons[val as usize])
        } else {
            format!("{}/val_{}.png", assets_dir, val)
        }
    }

    /// Get the highlighted image path (the _1 variant used by the game on hover).
    pub fn icon_src_highlight(&self, val: u8, assets_dir: &str) -> String {
        if !self.icons.is_empty() && (val as usize) < self.icons.len() {
            format!("{}/{}_1.gif", assets_dir, self.icons[val as usize])
        } else {
            format!("{}/val_{}.png", assets_dir, val)
        }
    }
}

/// Generate an HTML solution guide.
pub fn generate_html_guide(
    puzzle: &PuzzleJson,
    game: &Game,
    solution: &[(usize, usize)],
    assets_dir: &str,
) -> String {
    let m = puzzle.m;
    let h = puzzle.rows as usize;
    let w = puzzle.columns as usize;
    let pieces = game.pieces();

    let mut board = game.board().clone();
    let mut html = String::new();

    html.push_str(&format!(r#"<!DOCTYPE html>
<html>
<head>
<meta charset="UTF-8">
<title>Shapeshifter Level {} Solution</title>
<style>
body {{ font-family: 'Segoe UI', Arial, sans-serif; background: #fff; color: #222; max-width: 800px; margin: 0 auto; padding: 20px; }}
h1 {{ color: #333; text-align: center; }}
h2 {{ color: #444; border-bottom: 1px solid #ccc; padding-bottom: 5px; }}
.step {{ background: #f5f5f5; border-radius: 8px; padding: 15px; margin: 15px 0; border: 1px solid #ddd; }}
.board {{ display: inline-grid; grid-template-columns: repeat({w}, 50px); gap: 0; padding: 0; }}
.cell {{ width: 50px; height: 50px; position: relative; line-height: 0; }}
.cell img {{ width: 50px; height: 50px; display: block; }}
.cell.click-here {{ outline: 4px solid #2ecc40; outline-offset: -4px; }}
.info {{ color: #666; font-size: 14px; margin: 5px 0; }}
.solved {{ color: #4caf50; font-size: 24px; text-align: center; font-weight: bold; padding: 20px; }}
.arrow {{ text-align: center; font-size: 24px; color: #999; }}
</style>
</head>
<body>
<h1>Shapeshifter Level {} Solution</h1>
<p class="info" style="text-align:center">{h}&times;{w} board, M={m}, {n} pieces</p>
"#, puzzle.level, puzzle.level, h = h, w = w, m = m, n = pieces.len()));

    // First step shows the initial board with the piece highlight, so no need for a separate initial board.

    for (i, &(row, col)) in solution.iter().enumerate() {
        let piece = &pieces[i];
        let mask = piece.placed_at(row, col);

        html.push_str(&format!("<div class=\"step\"><h2>Piece {}</h2>\n", i));

        // Board with highlight and click marker
        html.push_str(&render_board(&board, h, w, puzzle, assets_dir, Some(mask), Some((row, col))));

        board.apply_piece(mask);
        html.push_str("</div>\n");
    }

    if board.is_solved() {
        html.push_str("<div class=\"step\"><h2>Result</h2>\n");
        html.push_str(&render_board(&board, h, w, puzzle, assets_dir, None, None));
        html.push_str("<div class=\"solved\">SOLVED!</div>\n</div>\n");
    }

    html.push_str("</body></html>");
    html
}

fn render_board(
    board: &crate::core::board::Board,
    h: usize,
    w: usize,
    puzzle: &PuzzleJson,
    assets_dir: &str,
    piece_mask: Option<crate::core::bitboard::Bitboard>,
    click_pos: Option<(usize, usize)>,
) -> String {
    let mut s = format!("<div class=\"board\" style=\"grid-template-columns: repeat({}, 50px)\">\n", w);
    for r in 0..h {
        for c in 0..w {
            let val = board.get(r, c);
            let bit = (r * 15 + c) as u32;
            let is_piece = piece_mask.map_or(false, |m| m.get_bit(bit));
            let is_click = click_pos.map_or(false, |(cr, cc)| r == cr && c == cc);
            let class = match (is_click, is_piece) {
                (true, _) => "cell highlight click-here",
                (false, true) => "cell highlight",
                _ => "cell",
            };
            let src = if is_piece {
                puzzle.icon_src_highlight(val, assets_dir)
            } else {
                puzzle.icon_src(val, assets_dir)
            };
            s.push_str(&format!("<div class=\"{}\"><img src=\"{}\"></div>\n", class, src));
        }
    }
    s.push_str("</div>\n");
    s
}
