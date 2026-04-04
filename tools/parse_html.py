#!/usr/bin/env python3
"""Parse a Neopets Shapeshifter HTML page into a JSON puzzle file."""

import json
import re
import sys
from html.parser import HTMLParser


def parse_shapeshifter_html(html: str) -> dict:
    # Extract level number
    level_match = re.search(r'LEVEL\s+(\d+)', html)
    level = int(level_match.group(1)) if level_match else 0

    # Extract board dimensions and cell values from imgLocStr
    # imgLocStr[x][y] = "swo" or "hel" etc.
    img_entries = re.findall(r'imgLocStr\[(\d+)\]\[(\d+)\]\s*=\s*"(\w+)"', html)

    if 'You Won!' in html:
        raise ValueError("This is a 'You Won!' page, not an active puzzle. Save the page BEFORE clicking to solve.")

    gx_match = re.search(r'gX\s*=\s*(\d+)', html)
    gy_match = re.search(r'gY\s*=\s*(\d+)', html)
    if not gx_match or not gy_match:
        raise ValueError("Could not find board dimensions (gX/gY) in HTML. Is this a Shapeshifter puzzle page?")
    gx = int(gx_match.group(1))  # columns
    gy = int(gy_match.group(1))  # rows

    # Collect unique icon names to determine M and value mapping
    icons = set()
    cell_map = {}
    for x, y, icon in img_entries:
        cell_map[(int(x), int(y))] = icon
        icons.add(icon)

    # Determine value cycle from the goal display:
    # The HTML shows: icon -> arrow -> GOAL -> arrow -> icon
    # Pattern: hel_0 -> swo_0 (GOAL) -> hel_0 means hel->swo->hel, so M=2
    # We need to find the cycle. Look for the goal section.
    goal_match = re.findall(
        r'<img src="[^"]*?/(\w+)_0\.gif"[^>]*>\s*(?:<br><b><small>GOAL</small></b>)?',
        html[html.find('GOAL')-200:html.find('GOAL')+200] if 'GOAL' in html else ''
    )

    # Parse the icon cycle from around the GOAL marker.
    # The display shows: icon_1 -> GOAL_icon -> icon_3 -> ...
    # The GOAL icon = value 0 (deficit 0, already solved). Each placement decrements
    # a cell's deficit by 1 mod M, cycling it to the next icon in the chain.
    # The icons around GOAL: [..., val=M-1, GOAL=0, val=1, ...]
    # Find the goal cycle table: it contains the GOAL text and arrow icons.
    # Search backwards from GOAL to find the enclosing table.
    goal_pos = html.find('GOAL')
    cycle_section = ''
    if goal_pos >= 0:
        # Look back for the nearest <table (use larger window for long cycle rows)
        search_start = max(0, goal_pos - 2000)
        table_start = html.rfind('<table', search_start, goal_pos)
        if table_start >= 0:
            table_end = html.find('</table>', goal_pos)
            if table_end >= 0:
                cycle_section = html[table_start:table_end + 10]
    cycle_icons = re.findall(r'/(\w+)_0\.gif', cycle_section)
    cycle_icons = [i for i in cycle_icons if i != 'arrow']

    if cycle_icons:
        # Find which icon is the GOAL (has the GOAL label after it)
        # Search the cycle section for the icon directly before the GOAL label
        goal_icon_match = re.search(r'/(\w+)_0\.gif[^>]*>[^<]*<br><b><small>GOAL', cycle_section, re.DOTALL)
        if goal_icon_match:
            goal_icon = goal_icon_match.group(1)
        else:
            goal_icon = cycle_icons[len(cycle_icons) // 2]  # middle is usually GOAL

        # The cycle sequence has a repeated icon at the end (wraps around).
        # e.g. [gob, cro, swo, gob] means the cycle is gob->cro->swo->gob, M=3.
        # Remove the trailing duplicate to get the unique cycle.
        if len(cycle_icons) > 1 and cycle_icons[-1] == cycle_icons[0]:
            cycle_icons = cycle_icons[:-1]
        m = len(cycle_icons)
        # Find goal_icon position in cycle
        goal_pos = cycle_icons.index(goal_icon)
        # Build mapping: starting from GOAL position, assign values 0, 1, 2, ...
        icon_to_val = {}
        for offset in range(m):
            idx = (goal_pos + offset) % m
            icon = cycle_icons[idx]
            icon_to_val[icon] = offset
    else:
        sorted_icons = sorted(icons)
        m = len(sorted_icons)
        icon_to_val = {icon: i for i, icon in enumerate(sorted_icons)}

    # Build board grid (note: imgLocStr uses [x][y] = [col][row])
    board = []
    for row in range(gy):
        board_row = []
        for col in range(gx):
            icon = cell_map.get((col, row), list(icons)[0])
            board_row.append(icon_to_val.get(icon, 0))
        board.append(board_row)

    # Parse piece shapes from the HTML tables
    # Active shape and next shapes are in tables with square.gif
    # Find "ACTIVE SHAPE" section
    active_pos = html.find('ACTIVE SHAPE')
    next_pos = html.find('NEXT SHAPES')

    def parse_shape_tables(section: str) -> list:
        """Parse shape tables from an HTML section. Each shape is a table with
        <td> cells that either contain square.gif (filled) or are empty (unfilled)."""
        shapes = []

        # Find inner tables (shape grids) - they have cellpadding=0 cellspacing=0
        # Handle both quoted and unquoted attribute values
        table_pattern = r'<table\s+border=.?0.?\s+cellpadding=.?0.?\s+cellspacing=.?0.?>(.*?)</table>'
        tables = re.findall(table_pattern, section, re.DOTALL)

        for table_html in tables:
            rows = re.findall(r'<tr>(.*?)</tr>', table_html, re.DOTALL)
            shape = []
            for row_html in rows:
                cells = re.findall(r'<td[^>]*>(.*?)</td>', row_html, re.DOTALL)
                shape_row = []
                for cell_html in cells:
                    filled = 'square.gif' in cell_html
                    shape_row.append(filled)
                if shape_row:
                    shape.append(shape_row)
            if shape:
                shapes.append(shape)

        return shapes

    pieces = []

    # Parse active shape
    if active_pos >= 0:
        if next_pos >= 0:
            active_section = html[active_pos:next_pos]
        else:
            active_section = html[active_pos:active_pos + 2000]
        active_shapes = parse_shape_tables(active_section)
        pieces.extend(active_shapes)

    # Parse next shapes — find the end by looking for the next major section
    if next_pos >= 0:
        # End at "rules_icon" or "Back to Games" or end of relevant content
        end_markers = ['rules_icon', 'Back to Games', 'shapeshifter_instruct']
        next_end = len(html)
        for marker in end_markers:
            pos = html.find(marker, next_pos)
            if pos >= 0 and pos < next_end:
                next_end = pos
        next_section = html[next_pos:next_end]
        next_shapes = parse_shape_tables(next_section)
        pieces.extend(next_shapes)

    # Build icon list ordered by value: icons[0] = goal icon, icons[1] = next, etc.
    icon_list = [""] * m
    for icon, val in icon_to_val.items():
        icon_list[val] = icon

    return {
        "level": level,
        "m": m,
        "rows": gy,
        "columns": gx,
        "board": board,
        "pieces": pieces,
        "icons": icon_list,
    }


def main():
    if len(sys.argv) < 2:
        print(f"Usage: {sys.argv[0]} <input.html> [output.json]")
        sys.exit(1)

    input_path = sys.argv[1]
    output_path = sys.argv[2] if len(sys.argv) > 2 else input_path.rsplit('.', 1)[0] + '.json'

    with open(input_path) as f:
        html = f.read()

    puzzle = parse_shapeshifter_html(html)

    with open(output_path, 'w') as f:
        json.dump(puzzle, f, indent=2)

    print(f"Parsed level {puzzle['level']}: {puzzle['rows']}x{puzzle['columns']}, M={puzzle['m']}, {len(puzzle['pieces'])} pieces")
    print(f"Board:")
    for row in puzzle['board']:
        print(f"  {row}")
    print(f"Pieces:")
    for i, piece in enumerate(puzzle['pieces']):
        print(f"  Piece {i}:")
        for row in piece:
            print(f"    {''.join('#' if c else '.' for c in row)}")
    print(f"Written to {output_path}")


if __name__ == '__main__':
    main()
