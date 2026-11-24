#!/usr/bin/env node
/**
 * build-audit-report.mjs — يولّد docs/spec/audit-findings.md من دفتر التدقيق،
 * أو يتحقّق من حداثته مع --check. بلا تبعيات خارجية.
 *   node scripts/build-audit-report.mjs [--check]
 */
import { readFileSync, writeFileSync, existsSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

/** جذر المشروع = أوّل مجلد صاعد يحتوي docs/spec (سواء شُغِّل من scripts/ أو من مجلد الاختبار). */
function findRoot(start) {
  let dir = start;
  for (;;) {
    if (existsSync(join(dir, "docs", "spec"))) return dir;
    const up = dirname(dir);
    if (up === dir) return start;
    dir = up;
  }
}
const ROOT = findRoot(fileURLToPath(new URL(".", import.meta.url)));
const LEDGER = join(ROOT, "docs/spec/audit-ledger.json");
const OUT = join(ROOT, "docs/spec/audit-findings.md");
const CHECK = process.argv.includes("--check");

const DIMENSIONS = ["behavior", "data", "ui", "i18n", "safety", "performance", "packaging", "deps", "cli", "lessons", "tests"];
const METHODS = ["code", "test", "browser", "run", "registry"];
const VERDICTS = ["match", "documented-deviation", "undocumented-deviation", "unverifiable"];
const SEVERITIES = ["blocker", "high", "medium", "low", "info"];
const SEV_ORDER = { blocker: 0, high: 1, medium: 2, low: 3, info: 4 };

function fail(msg) {
  process.stderr.write(`audit: ${msg}\n`);
  process.exit(1);
}

const ledger = JSON.parse(readFileSync(LEDGER, "utf8"));
if (!ledger || !Array.isArray(ledger.units)) fail("audit-ledger.json must have a units[] array");

const seen = new Set();
ledger.units.forEach((u, i) => {
  const at = `units[${i}]`;
  if (!/^A-\d{4}$/.test(u.id || "")) fail(`${at}.id must match A-NNNN`);
  if (seen.has(u.id)) fail(`${at}.id duplicate: ${u.id}`);
  seen.add(u.id);
  if (!DIMENSIONS.includes(u.dimension)) fail(`${at}.dimension invalid: ${u.dimension}`);
  for (const key of ["old_ref", "new_ref", "method", "verdict", "severity", "evidence"]) {
    if (typeof u[key] !== "string" || !u[key].trim()) fail(`${at}.${key} is required`);
  }
  if (!METHODS.includes(u.method)) fail(`${at}.method invalid: ${u.method}`);
  if (!VERDICTS.includes(u.verdict)) fail(`${at}.verdict invalid: ${u.verdict}`);
  if (!SEVERITIES.includes(u.severity)) fail(`${at}.severity invalid: ${u.severity}`);
  if (u.spec_id != null && (typeof u.spec_id !== "string" || !u.spec_id.trim())) fail(`${at}.spec_id must be a non-empty string or null`);
  if (u.method === "browser" && !/console/i.test(u.evidence)) fail(`${at}: browser evidence must mention console`);
  if (u.method === "registry" && !/license|ترخيص/i.test(u.evidence)) fail(`${at}: registry evidence must mention the license`);
  if (u.verdict === "undocumented-deviation" && SEV_ORDER[u.severity] > 2) {
    fail(`${at}: undocumented-deviation must be blocker/high/medium`);
  }
});

const groups = new Map();
for (const u of ledger.units) {
  if (!groups.has(u.dimension)) groups.set(u.dimension, []);
  groups.get(u.dimension).push(u);
}
const findings = ledger.units
  .filter((u) => u.verdict === "undocumented-deviation")
  .sort((a, b) => SEV_ORDER[a.severity] - SEV_ORDER[b.severity] || a.id.localeCompare(b.id));
const unverifiable = ledger.units.filter((u) => u.verdict === "unverifiable");
const documented = ledger.units.filter((u) => u.verdict === "documented-deviation");

const L = [];
L.push("# تقرير التدقيق الشامل — القديم ↔ الجديد");
L.push("");
L.push(`> BASE: \`${ledger.baseCommit}\` · HEAD: \`${ledger.headCommit}\``);
L.push("> مُولَّد من `docs/spec/audit-ledger.json` — لا تُعدِّله يدويًّا.");
L.push("");
L.push("## الملخّص");
L.push("");
L.push(`- إجمالي الوحدات: **${ledger.units.length}**`);
L.push(`- مطابق: ${ledger.units.filter((u) => u.verdict === "match").length}`);
L.push(`- انحراف موثّق: ${documented.length}`);
L.push(`- انحراف غير موثّق (findings): **${findings.length}**`);
L.push(`- غير قابل للتحقّق: ${unverifiable.length}`);
L.push("");
L.push("## Findings (مرتّبة بالخطورة)");
L.push("");
if (!findings.length) L.push("لا توجد انحرافات غير موثّقة.");
for (const u of findings) {
  L.push(`### ${u.id} — ${u.severity} — ${u.dimension}${u.spec_id ? ` (${u.spec_id})` : ""}`);
  L.push("");
  L.push(`- القديم: \`${u.old_ref}\``);
  L.push(`- الجديد: \`${u.new_ref}\``);
  L.push(`- method: \`${u.method}\` · reviewer: \`${u.reviewer}\``);
  L.push(`- الانحراف: ${u.deviation || "—"}`);
  L.push(`- الدليل: ${u.evidence}`);
  L.push("");
}
L.push("## حسب البُعد");
L.push("");
L.push("| البُعد | الوحدات | مطابق | موثّق | findings | غير مُتحقَّق |");
L.push("|---|---|---|---|---|---|");
for (const [dim, rows] of [...groups].sort((a, b) => a[0].localeCompare(b[0]))) {
  const c = (v) => rows.filter((u) => u.verdict === v).length;
  L.push(`| ${dim} | ${rows.length} | ${c("match")} | ${c("documented-deviation")} | ${c("undocumented-deviation")} | ${c("unverifiable")} |`);
}
if (unverifiable.length) {
  L.push("", "## غير قابل للتحقّق", "");
  for (const u of unverifiable) L.push(`- ${u.id} (${u.dimension}): ${u.evidence}`);
}
L.push("", "## الفهرس (كل الوحدات)", "");
L.push("| المعرّف | البُعد | spec | الحكم | الخطورة |");
L.push("|---|---|---|---|---|");
for (const u of ledger.units) {
  L.push(`| ${u.id} | ${u.dimension} | ${u.spec_id || "—"} | ${u.verdict} | ${u.severity} |`);
}
L.push("");
const md = L.join("\n");
if (CHECK) {
  let current = "";
  try { current = readFileSync(OUT, "utf8"); } catch { fail("audit-findings.md is stale (missing)"); }
  if (current !== md) fail("audit-findings.md is stale — run pnpm audit:report");
  process.stdout.write("audit: findings up to date\n");
} else {
  writeFileSync(OUT, md);
  process.stdout.write(`audit: wrote ${OUT} (${ledger.units.length} units)\n`);
}
