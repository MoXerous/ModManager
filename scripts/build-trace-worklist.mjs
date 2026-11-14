// Builds the traceability work-list for the rename pass.
// Usage: node build-trace-worklist.mjs <outDir>
import { readFileSync, writeFileSync, readdirSync, mkdirSync } from "node:fs";
import { join } from "node:path";

const ROOT = "E:/ModManager";
const out = process.argv[2];
mkdirSync(out, { recursive: true });

// 1) unnamed #[test] functions in Rust sources
const walk = (dir, outFiles = []) => {
  for (const e of readdirSync(dir, { withFileTypes: true })) {
    const p = join(dir, e.name);
    if (e.isDirectory()) walk(p, outFiles);
    else if (e.name.endsWith(".rs")) outFiles.push(p);
  }
  return outFiles;
};
const tests = [];
for (const f of walk(join(ROOT, "core/src")).concat(walk(join(ROOT, "src-tauri/src")), walk(join(ROOT, "cli/src")))) {
  const lines = readFileSync(f, "utf8").split(/\r?\n/);
  for (let i = 0; i < lines.length; i += 1) {
    if (!/^\s*#\[(tokio::)?test\]/.test(lines[i])) continue;
    for (let j = i + 1; j < Math.min(i + 4, lines.length); j += 1) {
      const m = lines[j].match(/fn\s+([a-z0-9_]+)\s*\(/);
      if (!m) break;
      if (!/^(spec|test)_f\d{3}/.test(m[1])) {
        tests.push({ file: f.replace(ROOT + "/", ""), line: j + 1, name: m[1] });
      }
      break;
    }
  }
}

// 2) untraceable Keep specs (name + domain + phase) from the generated matrix
const parity = readFileSync(join(ROOT, "docs/spec/parity.md"), "utf8").split(/\r?\n/);
const inHygiene = parity.findIndex((l) => l.includes("قائمة hygiene"));
const rows = [];
for (let i = inHygiene; i < parity.length; i += 1) {
  const m = parity[i].match(/^\|\s*\*\*(F-\d{3})\*\*\s*\|\s*(.+?)\s*\|\s*(.+?)\s*\|\s*(.+?)\s*\|$/);
  if (m) rows.push({ id: m[1], name: m[2].replace(/\*\*/g, ""), domain: m[3], phase: m[4] });
  else if (rows.length && parity[i].startsWith("## ")) break;
}

writeFileSync(join(out, "unnamed-tests.txt"), tests.map((t) => `${t.file}:${t.line}  ${t.name}`).join("\n"), "utf8");
writeFileSync(
  join(out, "untraceable-specs.md"),
  ["# بنود `Keep` بلا أثر قابلٍ للتتبّع (من `docs/spec/parity.md`)", "", "| المعرّف | البند | المجال | المرحلة |", "|---|---|---|---|", ...rows.map((r) => `| **${r.id}** | ${r.name} | ${r.domain} | ${r.phase} |`), ""].join("\n"),
  "utf8"
);
console.log(`unnamed tests: ${tests.length} | untraceable specs: ${rows.length}`);
console.log(`written: ${out}`);
