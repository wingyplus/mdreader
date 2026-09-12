// An example custom script: draws ```chart blocks as bar charts.
//
//     mdreader examples --script examples/custom-script.js

const escapeHtml = (text) =>
  text.replace(/[&<>]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;" })[c]);

// Reads the `label: value` lines of a chart block, ignoring anything else.
const readRows = (source) =>
  source
    .split("\n")
    .map((line) => line.split(/:(.*)/s))
    .map(([label, value]) => ({ label: label.trim(), value: Number(value) }))
    .filter(({ label, value }) => label !== "" && Number.isFinite(value));

const barChart = (rows, theme) => {
  const width = 520;
  const rowHeight = 26;
  const labelWidth = 130;
  const height = rows.length * rowHeight;
  const longest = Math.max(...rows.map(({ value }) => value), 0);
  const bar = theme === "dark" ? "#4493f8" : "#0550ae";
  const bars = rows.map(({ label, value }, i) => {
    const y = i * rowHeight;
    const length = longest === 0 ? 0 : ((width - labelWidth - 40) * value) / longest;
    return `
      <text x="${labelWidth - 8}" y="${y + 17}" text-anchor="end" fill="currentColor"
        font-size="13">${escapeHtml(label)}</text>
      <rect x="${labelWidth}" y="${y + 5}" width="${length}" height="16" rx="2" fill="${bar}" />
      <text x="${labelWidth + length + 6}" y="${y + 17}" fill="currentColor" font-size="13"
        opacity="0.7">${value}</text>`;
  });
  return `<svg viewBox="0 0 ${width} ${height}" width="100%" height="${height}"
    font-family="system-ui, sans-serif" role="img">${bars.join("")}</svg>`;
};

// Runs on load, after every page update, and whenever the theme changes.
mdreader.onRender((main, { theme }) => {
  for (const block of main.querySelectorAll("pre.lang-chart")) {
    const rows = readRows(mdreader.source(block));
    // A block that holds nothing to draw keeps showing its source.
    if (rows.length === 0) continue;
    block.innerHTML = barChart(rows, theme);
    block.classList.add("rendered");
  }
});
