#!/usr/bin/env node
import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, copyFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const HERE = dirname(fileURLToPath(import.meta.url));
const GEN = join(HERE, "build-audit-report.mjs");
const TMP = mkdtempSync(join(tmpdir(), "audit-report-"));
const SPEC_DIR = join(TMP, "docs", "spec");
mkdirSync(SPEC_DIR, { recursive: true });
copyFileSync(GEN, join(TMP, "build-audit-report.mjs"));

const unit = (over = {}) => ({
  id: "A-0001", dimension: "behavior", spec_id: "F-001",
  old_ref: "app/providers/modrinth_client.py::search_projects",
  new_ref: "core/src/providers/modrinth.rs::search_projects",
  method: "code", verdict: "match", severity: "info",
  deviation: null, evidence: "قراءة الكود: التوقيع والسلوك متطابقان", reviewer: "deepseek",
  ...over,
});
const writeLedger = (units) => writeFileSync(
  join(SPEC_DIR, "audit-ledger.json"),
  JSON.stringify({ version: 1, baseCommit: "4f71f18", headCommit: "HEAD", units }, null, 2),
);
const run = (...args) => spawnSync(process.execPath, [join(TMP, "build-audit-report.mjs"), ...args], { encoding: "utf8" });

try {
  // 1) مولّد صالح
  writeLedger([unit()]);
  assert.equal(run().status, 0);
  const md = readFileSync(join(SPEC_DIR, "audit-findings.md"), "utf8");
  assert.match(md, /# تقرير التدقيق الشامل/);
  assert.match(md, /A-0001/);

  // 2) وحدة بلا old_ref تُرفض
  const bad = unit(); delete bad.old_ref;
  writeLedger([bad]);
  const r2 = run();
  assert.notEqual(r2.status, 0);
  assert.match(r2.stderr, /old_ref is required/);

  // 3) حكم غير معروف يُرفض
  writeLedger([unit({ verdict: "maybe" })]);
  const r3 = run();
  assert.notEqual(r3.status, 0);
  assert.match(r3.stderr, /verdict invalid/);

  // 4) method=browser بلا console يُرفض
  writeLedger([unit({ method: "browser", evidence: "لقطة فقط" })]);
  const r4 = run();
  assert.notEqual(r4.status, 0);
  assert.match(r4.stderr, /browser evidence must mention console/);

  // 5) --check يفشل عند قِدَم المخرَج
  writeLedger([unit()]); run();
  writeLedger([unit(), unit({ id: "A-0002", spec_id: "F-002" })]);
  const r5 = run("--check");
  assert.notEqual(r5.status, 0);
  assert.match(r5.stderr, /stale/);

  console.log("audit-report tests: 5 passed");
} finally {
  rmSync(TMP, { recursive: true, force: true });
}
