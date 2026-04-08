// Port of tools/parse_html.py — parse Neopets Shapeshifter HTML into puzzle JSON.

export function parseShapeshifterHtml(html) {
    const levelMatch = html.match(/LEVEL\s+(\d+)/);
    const level = levelMatch ? parseInt(levelMatch[1]) : 0;

    if (html.includes('You Won!')) {
        throw new Error("This is a 'You Won!' page. Save the page BEFORE clicking to solve.");
    }

    const gxMatch = html.match(/gX\s*=\s*(\d+)/);
    const gyMatch = html.match(/gY\s*=\s*(\d+)/);
    if (!gxMatch || !gyMatch) {
        throw new Error('Could not find board dimensions (gX/gY). Is this a Shapeshifter page?');
    }
    const gx = parseInt(gxMatch[1]); // columns
    const gy = parseInt(gyMatch[1]); // rows

    // Extract cell icons: imgLocStr[col][row] = "iconname"
    const imgEntries = [...html.matchAll(/imgLocStr\[(\d+)\]\[(\d+)\]\s*=\s*"(\w+)"/g)];
    const cellMap = {};
    const icons = new Set();
    for (const [, x, y, icon] of imgEntries) {
        cellMap[`${x},${y}`] = icon;
        icons.add(icon);
    }

    // Parse icon cycle from GOAL section
    let m, iconToVal = {};
    const goalPos = html.indexOf('GOAL');
    if (goalPos >= 0) {
        const searchStart = Math.max(0, goalPos - 2000);
        const tableStart = html.lastIndexOf('<table', goalPos);
        const tableEnd = html.indexOf('</table>', goalPos);
        if (tableStart >= searchStart && tableEnd >= 0) {
            const cycleSection = html.slice(tableStart, tableEnd + 10);
            let cycleIcons = [...cycleSection.matchAll(/\/(\w+)_0\.gif/g)]
                .map(m => m[1])
                .filter(i => i !== 'arrow');

            // Find goal icon
            const goalIconMatch = cycleSection.match(/\/(\w+)_0\.gif[^>]*>[^<]*<br><b><small>GOAL/s);
            const goalIcon = goalIconMatch ? goalIconMatch[1] : cycleIcons[Math.floor(cycleIcons.length / 2)];

            // Remove trailing duplicate (wrap)
            if (cycleIcons.length > 1 && cycleIcons[cycleIcons.length - 1] === cycleIcons[0]) {
                cycleIcons = cycleIcons.slice(0, -1);
            }

            m = cycleIcons.length;
            const goalIdx = cycleIcons.indexOf(goalIcon);
            for (let offset = 0; offset < m; offset++) {
                const idx = (goalIdx + offset) % m;
                iconToVal[cycleIcons[idx]] = (m - offset) % m;
            }
        }
    }

    if (!m) {
        const sortedIcons = [...icons].sort();
        m = sortedIcons.length;
        sortedIcons.forEach((icon, i) => { iconToVal[icon] = i; });
    }

    // Build board grid (imgLocStr uses [col][row])
    const board = [];
    for (let row = 0; row < gy; row++) {
        const boardRow = [];
        for (let col = 0; col < gx; col++) {
            const icon = cellMap[`${col},${row}`] || [...icons][0];
            boardRow.push(iconToVal[icon] || 0);
        }
        board.push(boardRow);
    }

    // Parse piece shapes from HTML tables
    function parseShapeTables(section) {
        const shapes = [];
        const tableRegex = /<table\s+border=.?0.?\s+cellpadding=.?0.?\s+cellspacing=.?0.?>(.*?)<\/table>/gs;
        let tableMatch;
        while ((tableMatch = tableRegex.exec(section)) !== null) {
            const tableHtml = tableMatch[1];
            const rows = [...tableHtml.matchAll(/<tr>(.*?)<\/tr>/gs)];
            const shape = [];
            for (const [, rowHtml] of rows) {
                const cells = [...rowHtml.matchAll(/<td[^>]*>(.*?)<\/td>/gs)];
                const shapeRow = cells.map(([, cellHtml]) => cellHtml.includes('square.gif'));
                if (shapeRow.length > 0) shape.push(shapeRow);
            }
            if (shape.length > 0) shapes.push(shape);
        }
        return shapes;
    }

    const pieces = [];
    const activePos = html.indexOf('ACTIVE SHAPE');
    const nextPos = html.indexOf('NEXT SHAPES');

    if (activePos >= 0) {
        const activeEnd = nextPos >= 0 ? nextPos : activePos + 2000;
        pieces.push(...parseShapeTables(html.slice(activePos, activeEnd)));
    }

    if (nextPos >= 0) {
        const endMarkers = ['rules_icon', 'Back to Games', 'shapeshifter_instruct'];
        let nextEnd = html.length;
        for (const marker of endMarkers) {
            const pos = html.indexOf(marker, nextPos);
            if (pos >= 0 && pos < nextEnd) nextEnd = pos;
        }
        pieces.push(...parseShapeTables(html.slice(nextPos, nextEnd)));
    }

    // Build icon list ordered by deficit
    const iconList = Array(m).fill('');
    for (const [icon, val] of Object.entries(iconToVal)) {
        iconList[val] = icon;
    }

    return { level, m, rows: gy, columns: gx, board, pieces, icons: iconList };
}
