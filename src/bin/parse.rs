//! Parse a Neopets Shapeshifter HTML page into puzzle JSON.
//!
//! Replaces the Python parse_html.py script with a native Rust implementation.
//! Reads HTML from a file argument or stdin, writes JSON to stdout or a file.

use regex::Regex;
use shapeshifter::puzzle::PuzzleJson;

fn parse_shapeshifter_html(html: &str) -> PuzzleJson {
    // Extract level number.
    let level = Regex::new(r"LEVEL\s+(\d+)")
        .unwrap()
        .captures(html)
        .map(|c| c[1].parse::<u32>().unwrap())
        .unwrap_or(0);

    if html.contains("You Won!") {
        eprintln!("Error: This is a 'You Won!' page, not an active puzzle.");
        eprintln!("Save the page BEFORE clicking to solve.");
        std::process::exit(1);
    }

    // Extract board dimensions.
    let gx: usize = Regex::new(r"gX\s*=\s*(\d+)")
        .unwrap()
        .captures(html)
        .expect("Could not find gX in HTML")[1]
        .parse()
        .unwrap();
    let gy: usize = Regex::new(r"gY\s*=\s*(\d+)")
        .unwrap()
        .captures(html)
        .expect("Could not find gY in HTML")[1]
        .parse()
        .unwrap();

    // Extract cell icon assignments: imgLocStr[col][row] = "icon"
    let img_re = Regex::new(r#"imgLocStr\[(\d+)\]\[(\d+)\]\s*=\s*"(\w+)""#).unwrap();
    let mut cell_map = std::collections::HashMap::new();
    for cap in img_re.captures_iter(html) {
        let col: usize = cap[1].parse().unwrap();
        let row: usize = cap[2].parse().unwrap();
        let icon = cap[3].to_string();
        cell_map.insert((col, row), icon);
    }

    // Parse icon cycle from the GOAL section.
    let (m, icon_to_val, icon_list) = parse_icon_cycle(html);

    // Build board grid.
    let mut board = Vec::with_capacity(gy);
    for row in 0..gy {
        let mut board_row = Vec::with_capacity(gx);
        for col in 0..gx {
            let val = cell_map
                .get(&(col, row))
                .and_then(|icon| icon_to_val.get(icon.as_str()))
                .copied()
                .unwrap_or(0);
            board_row.push(val);
        }
        board.push(board_row);
    }

    // Parse piece shapes.
    let pieces = parse_pieces(html);

    PuzzleJson {
        level,
        m,
        rows: gy as u8,
        columns: gx as u8,
        board,
        pieces,
        icons: icon_list,
    }
}

fn parse_icon_cycle(html: &str) -> (u8, std::collections::HashMap<&str, u8>, Vec<String>) {
    let goal_pos = html.find("GOAL");

    if let Some(gp) = goal_pos {
        // Find the table containing the GOAL marker.
        let search_start = gp.saturating_sub(2000);
        let table_start = html[search_start..gp].rfind("<table").map(|p| p + search_start);
        let table_end = html[gp..].find("</table>").map(|p| p + gp + 8);

        if let (Some(start), Some(end)) = (table_start, table_end) {
            let cycle_section = &html[start..end];
            let icon_re = Regex::new(r"/(\w+)_0\.gif").unwrap();
            let mut cycle_icons: Vec<&str> = Vec::new();
            for cap in icon_re.captures_iter(cycle_section) {
                let name = cap.get(1).unwrap().as_str();
                if name != "arrow" {
                    cycle_icons.push(name);
                }
            }

            // Find the GOAL icon.
            let goal_icon_re =
                Regex::new(r"/(\w+)_0\.gif[^>]*>[^<]*<br><b><small>GOAL").unwrap();
            let goal_icon = goal_icon_re
                .captures(cycle_section)
                .map(|c| c.get(1).unwrap().as_str())
                .unwrap_or_else(|| cycle_icons[cycle_icons.len() / 2]);

            // Remove trailing wrap duplicate.
            if cycle_icons.len() > 1 && cycle_icons.last() == cycle_icons.first() {
                cycle_icons.pop();
            }

            let m = cycle_icons.len() as u8;
            let goal_idx = cycle_icons.iter().position(|&i| i == goal_icon).unwrap_or(0);

            let mut icon_to_val = std::collections::HashMap::new();
            let mut icon_list = vec![String::new(); m as usize];
            for offset in 0..m as usize {
                let idx = (goal_idx + offset) % m as usize;
                let icon = cycle_icons[idx];
                let val = ((m as usize - offset) % m as usize) as u8;
                icon_to_val.insert(icon, val);
                icon_list[val as usize] = icon.to_string();
            }

            return (m, icon_to_val, icon_list);
        }
    }

    // Fallback: collect all unique icons from imgLocStr.
    let img_re = Regex::new(r#"imgLocStr\[\d+\]\[\d+\]\s*=\s*"(\w+)""#).unwrap();
    let mut icons: Vec<&str> = img_re
        .captures_iter(html)
        .map(|c| c.get(1).unwrap().as_str())
        .collect();
    icons.sort();
    icons.dedup();
    let m = icons.len() as u8;
    let icon_to_val: std::collections::HashMap<&str, u8> =
        icons.iter().enumerate().map(|(i, &icon)| (icon, i as u8)).collect();
    let icon_list: Vec<String> = icons.iter().map(|s| s.to_string()).collect();
    (m, icon_to_val, icon_list)
}

fn parse_pieces(html: &str) -> Vec<Vec<Vec<bool>>> {
    let mut pieces = Vec::new();

    // Parse shapes from ACTIVE SHAPE and NEXT SHAPES sections.
    let active_pos = html.find("ACTIVE SHAPE");
    let next_pos = html.find("NEXT SHAPES");

    if let Some(ap) = active_pos {
        let end = next_pos.unwrap_or(ap + 2000).min(html.len());
        let section = &html[ap..end];
        pieces.extend(parse_shape_tables(section));
    }

    if let Some(np) = next_pos {
        let end_markers = ["rules_icon", "Back to Games", "shapeshifter_instruct"];
        let end = end_markers
            .iter()
            .filter_map(|m| html[np..].find(m).map(|p| p + np))
            .min()
            .unwrap_or(html.len());
        let section = &html[np..end];
        pieces.extend(parse_shape_tables(section));
    }

    pieces
}

fn parse_shape_tables(section: &str) -> Vec<Vec<Vec<bool>>> {
    let mut shapes = Vec::new();

    // Find inner shape tables (cellpadding=0 cellspacing=0).
    let table_re = Regex::new(
        r"(?si)<table\s+border=.?0.?\s+cellpadding=.?0.?\s+cellspacing=.?0.?>(.*?)</table>",
    )
    .unwrap();
    let row_re = Regex::new(r"(?si)<tr>(.*?)</tr>").unwrap();
    let cell_re = Regex::new(r"(?si)<td[^>]*>(.*?)</td>").unwrap();

    for table_cap in table_re.captures_iter(section) {
        let table_html = &table_cap[1];
        let mut shape: Vec<Vec<bool>> = Vec::new();

        for row_cap in row_re.captures_iter(table_html) {
            let row_html = &row_cap[1];
            let row: Vec<bool> = cell_re
                .captures_iter(row_html)
                .map(|c| c[1].contains("square.gif"))
                .collect();
            if !row.is_empty() {
                shape.push(row);
            }
        }

        if !shape.is_empty() {
            shapes.push(shape);
        }
    }

    shapes
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let mut input_path = None;
    let mut output_path = None;
    let mut history_path = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-o" | "--output" => {
                i += 1;
                output_path = Some(args[i].clone());
            }
            "--history" => {
                i += 1;
                history_path = Some(args[i].clone());
            }
            "-h" | "--help" => {
                eprintln!(
                    "Usage: parse [input.html] [OPTIONS]\n\n\
                     Parse a Neopets Shapeshifter HTML page into puzzle JSON.\n\
                     Reads HTML from a file argument or stdin.\n\n\
                     Options:\n  \
                       -o, --output PATH   Write JSON to PATH (default: stdout)\n  \
                       --history PATH      Append to JSONL history if new game\n  \
                       -h, --help          Show this help"
                );
                std::process::exit(0);
            }
            _ => {
                input_path = Some(args[i].clone());
            }
        }
        i += 1;
    }

    let html = if let Some(ref path) = input_path {
        std::fs::read_to_string(path).expect("failed to read HTML file")
    } else {
        use std::io::Read;
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf).expect("failed to read stdin");
        buf
    };

    let puzzle = parse_shapeshifter_html(&html);
    let json = serde_json::to_string(&puzzle).unwrap();

    if let Some(ref path) = output_path {
        std::fs::write(path, &json).expect("failed to write output");
        eprintln!(
            "Level {}: {}x{}, M={}, {} pieces → {}",
            puzzle.level, puzzle.rows, puzzle.columns, puzzle.m, puzzle.pieces.len(), path
        );
    } else {
        println!("{}", json);
    }

    // Append to history file if this is a new, complete game.
    if let Some(ref path) = history_path {
        use shapeshifter::level::get_level;

        let in_progress = get_level(puzzle.level)
            .is_some_and(|spec| puzzle.pieces.len() < spec.shapes as usize);

        if in_progress {
            eprintln!("Game already in progress (fewer pieces than expected). Skipping history.");
        } else {
            // Read existing history, append if not duplicate.
            let existing = std::fs::read_to_string(path).unwrap_or_default();
            if !existing.lines().any(|line| line == json) {
                use std::io::Write;
                let mut f = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                    .expect("failed to open history file");
                writeln!(f, "{}", json).expect("failed to write history");
                eprintln!("Appended to {}", path);
            } else {
                eprintln!("Already in history.");
            }
        }
    }
}
