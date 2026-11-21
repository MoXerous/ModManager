# Full Old-vs-New Parity Audit Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** إنتاج تدقيقٍ شامل يُعيد إثبات كل سلوك/بيانات/واجهة/أمان/أداء/تغليف/تبعية/اختبار في المشروع الجديد مقابل القديم عند `4f71f18`، وتسليم تقرير `findings` مرتّب بالخطورة بلا إصلاحات داخل الشريحة.

**Architecture:** دفترٌ واحد `docs/spec/audit-ledger.json` يتغذّى من أربعة مسارات **تسلسلية** (`ws1..ws4.json`، قرار المستخدم 2026-11-21 تغليبًا لقاعدة SDD)، ثم مراجعةٌ عدائية، ثم مولّدٌ حتمي `scripts/build-audit-report.mjs` يُنتج `audit-findings.md`. لا تُلمس مصادر التطبيق ولا المشروع القديم. الفحصُ الآلي يتحقّق من اكتمال الدفتر وحداثة المخرَج فقط، لا من غيابFinding.

**Tech Stack:** Node 22+ (ESM، `node:fs`/`node:assert`/`node:child_process`) · pnpm scripts · deepseek subagents · OpenCode embedded browser ·لا تبعيات جديدة.

## Global Constraints

- BASE = `D:\latest_project` @ `4f71f18736c95a1eeb877fe8363f11871118c618` — **قراءة فقط**.
- HEAD = `E:\ModManager` @ `git rev-parse HEAD`.
- لا `unwrap()`/`expect()` في كود الإنتاج (لا كود Rust هنا)؛ لا تبعيات جديدة.
- كل وحدة `AuditUnit` يجب أن تحمل `old_ref` و`new_ref` و`method` و`verdict` و`severity` و`evidence`؛ أي وحدة ناقصة تُفشل البوابة.
- `method=browser` ⇒ `evidence` يحتوي على كلمة `console` ويُرفق لقطة شاشة.
- `method=registry` ⇒ `evidence` يحتوي تاريخ الإصدار والترخيص والصيانة.
- `feature-parity.json` و`parity.md` و`feature-parity.md` **لا تُعدَّل** في هذه الشريحة.
- لغة المستندات: عربية مع مُعرّفات إنجليزية؛ لا مفاتيح خامّة في الواجهة (لا واجهة جديدة هنا).
- كل التزامات Conventional Commit؛ فرع العمل `docs/audit-full-parity` من `main`.

---

## File Structure

| الملف | المسؤولية |
|---|---|
| `docs/spec/audit-inventory.json` | جرد وحدات التدقيق (العدد النهائي + مصادرها) |
| `.opencode/analyses/audit/ws1.json` | مخرَّص مسار السلوك/البيانات |
| `.opencode/analyses/audit/ws2.json` | مخرَّص مسار الواجهة/الأمان/الأداء |
| `.opencode/analyses/audit/ws3.json` | مخرَّص مسار التغليف/التبعيات/CLI |
| `.opencode/analyses/audit/ws4.json` | مخرَّص مسار الدروس/الاختبارات |
| `docs/spec/audit-ledger.json` | الدفتر المدمج (مصدر الحقيقة) |
| `docs/spec/audit-findings.md` | **مولّد** من الدفتر |
| `docs/spec/audit-report.md` | ملخّص عربي بخطورة مرتّبة |
| `scripts/build-audit-report.mjs` | مولّد/فاحص الدفتر |
| `scripts/test-audit-report.mjs` | اختبار المولّد بلا تبعيات |
| `package.json` | سكربتات `audit:report` · `audit:check` · `audit:test` |
| `.github/workflows/ci.yml` | خطوة `pnpm audit:check` |

---

### Task 1: مخطّط الدفتر ومولّد التقرير

**Files:**
- Create: `scripts/build-audit-report.mjs`
- Create: `scripts/test-audit-report.mjs`
- Create: `docs/spec/audit-ledger.json`
- Modify: `package.json`

**Interfaces:**
- Consumes: لا شيء.
- Produces: `buildAuditReport()` سلوك CLI (كتابة/فحص)؛ مخطّط `AuditUnit` الذي ستنتجه Tasks 3–6؛ سكربت `pnpm audit:check`.

**انحرافات معتمدة من المستخدم (2026-11-21) بعد مراجعة المهمة:** (1) استبدال سطر `ROOT` ببحثٍ تصاعديّ `findRoot` + `fileURLToPath`؛ (2) إضافة قسم «الفهرس (كل الوحدات)» إلى المخرَج. كلاهما ظاهر في الكود أدناه، ولا يجوز نسخ السطر القديم في المهام التالية.

**ملاحظات صغرى مُسجَّلة للمراجعة النهائية (لا تمنع التنفيذ):** سطر فارغ قبل `## حسب البُعد` عند غياب findings؛ التحقق من حقل `reviewer` و`deviation`؛ تغليف `JSON.parse` برسالة `audit:`؛ عدَم سطر newline في الدفتر الابتدائي؛ اختبار مسار `scripts/` الفعلي؛ حالات اختبار إضافية (تكرار id، registry بلا ترخيص، undocumented-deviation بـlow، ونجاح `--check`).

- [ ] **Step 1: اكتب الاختبار الفاشل** — `scripts/test-audit-report.mjs`:

```js
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
```

- [ ] **Step 2: شغّل الاختبار وتأكد من فشله للسبب الصحيح**

Run: `node scripts/test-audit-report.mjs`
Expected: FAIL بـ `ENOENT: no such file or directory, copyFileSync ... build-audit-report.mjs`.

- [ ] **Step 3: اكتب المولّد** — `scripts/build-audit-report.mjs`:

```js
#!/usr/bin/env node
/**
 * build-audit-report.mjs — يولّد docs/spec/audit-findings.md من دفتر التدقيق،
 * أو يتحقّق من حداثته مع --check. بلا تبعيات خارجية.
 *   node scripts/build-audit-report.mjs [--check]
 */
import { readFileSync, writeFileSync, existsSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

/** انحراف معتمد من المستخدم 2026-11-21: بحث تصاعدي عن أول سلف يحتوي docs/spec.
 *  السطر الأصلي `new URL("..", import.meta.url)` يصعد مستوىً زائدًا عند نسخ المولّد إلى مجلد مؤقت. */
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
  if (u.spec_id != null && !/^F-\d{3}$/.test(u.spec_id)) fail(`${at}.spec_id must match F-NNN`);
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
L.push("| المعرّف | البُعد | spec | الحكم | الشدّة |");
L.push("|---|---|---|---|---|");
for (const u of ledger.units) {
  L.push(`| ${u.id} | ${u.dimension} | ${u.spec_id || "-"} | ${u.verdict} | ${u.severity} |`);
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
```

- [ ] **Step 4: شغّل الاختبار وتأكد من نجاحه**

Run: `node scripts/test-audit-report.mjs`
Expected: `audit-report tests: 5 passed`.

- [ ] **Step 5: أنشئ الدفتر الابتدائي وولّد المخرَج**

Run:
```bash
node -e "const fs=require('fs');fs.writeFileSync('docs/spec/audit-ledger.json',JSON.stringify({version:1,baseCommit:'4f71f18736c95a1eeb877fe8363f11871118c618',headCommit:require('child_process').execSync('git rev-parse HEAD',{encoding:'utf8'}).trim(),units:[]},null,2))"
node scripts/build-audit-report.mjs
node scripts/build-audit-report.mjs --check
```
Expected: `audit: wrote ... (0 units)` ثم `audit: findings up to date`.

- [ ] **Step 6: أضف سكربتات pnpm** — في `package.json` داخل `"scripts"`:

```json
"audit:report": "node scripts/build-audit-report.mjs",
"audit:check": "node scripts/build-audit-report.mjs --check",
"audit:test": "node scripts/test-audit-report.mjs",
```

- [ ] **Step 7: تحقّق واجمع**

Run: `pnpm audit:test && pnpm audit:check`
Expected: كلاهما يمرّ.
Commit: `git add scripts/build-audit-report.mjs scripts/test-audit-report.mjs docs/spec/audit-ledger.json docs/spec/audit-findings.md package.json && git commit -m "feat(audit): add the parity audit ledger and deterministic report generator"`

---

### Task 2: جرد الوحدات (Phase 0)

**Files:**
- Create: `docs/spec/audit-inventory.json`

**Interfaces:**
- Consumes: `docs/spec/features.md` · `docs/spec/screens.md` · `docs/spec/data-model.md` · `docs/spec/lessons.md` · `docs/spec/tests-to-port.md` · `D:\latest_project\tests\` · `D:\latest_project\packaging\`.
- Produces: العدد النهائي للوحدات لكل بُعد؛ تستخدمه Tasks 3–6 كـ`expectedCount`.

- [ ] **Step 1: اكتب الجرد** — `docs/spec/audit-inventory.json`:

```json
{
  "baseCommit": "4f71f18736c95a1eeb877fe8363f11871118c618",
  "dimensions": {
    "behavior": { "count": 181, "source": "docs/spec/features.md", "note": "F-001..F-181 — يُعاد إثبات كلٍّ منها" },
    "data": { "count": 12, "source": "docs/spec/data-model.md + app/storage + core/src/db" },
    "ui": { "count": 24, "source": "docs/spec/screens.md §3" },
    "i18n": { "count": 4, "source": "src/i18n/en.json + ar.json" },
    "safety": { "count": 10, "source": "core/src/safety + downloader + modpack" },
    "performance": { "count": 6, "source": "docs/spec/performance-budgets.md" },
    "packaging": { "count": 8, "source": "D:/latest_project/packaging (48 files) + .github/workflows" },
    "deps": { "count": 4, "source": "Cargo.toml + package.json" },
    "cli": { "count": 6, "source": "cli/ + src-tauri/ + src/lib/ipc.ts" },
    "lessons": { "count": 10, "source": "docs/spec/lessons.md §1..§10" },
    "tests": { "count": 8, "source": "34 old test files vs 24 Rust + 5 TS" }
  },
  "expectedTotal": 276
}
```

- [ ] **Step 2: تحقّق من العدد**

Run: `node -e "const i=require('./docs/spec/audit-inventory.json');const s=Object.values(i.dimensions).reduce((a,d)=>a+d.count,0);if(s!==i.expectedTotal)throw new Error(s+' != '+i.expectedTotal);console.log('inventory ok:',s)"`
Expected: `inventory ok: 276`.

- [ ] **Step 3: اجمع**

Commit: `git add docs/spec/audit-inventory.json && git commit -m "docs(audit): fix the audit unit inventory at 276 units"`

---

### Task 3: WS1 — السلوك والبيانات والتوافق

**Files:**
- Create: `.opencode/analyses/audit/ws1.json`

**Interfaces:**
- Consumes: مخطّط `AuditUnit` من Task 1؛ `audit-inventory.json` (behavior=181، data=12).
- Produces: `ws1.json` بالصيغة `{"dimensionCounts":{...},"units":[...]}` — يُستهلك في Task 7.

- [ ] **Step 1: شغّل الوكيل** — subagent `general` بنموذج `opencode/deepseek-v4.1-flash-free`:

```text
أنت مسار WS1 في تدقيق المطابقة الشامل لمشروع ModManager. للقراءة فقط: لا تعدّل أي ملف مصدر.
المرجع القديم BASE: D:\latest_project @ 4f71f18 (قراءة فقط). المرجع الجديد HEAD: E:\ModManager.
اقرأ: E:\ModManager\.opencode\analyses\audit\prompts\ws1-brief.md (سيُكتب في الخطوة 2) ثم نفّذ ما فيه حرفيًا.
أخرِج ملفًا واحدًا: E:\ModManager\.opencode\analyses\audit\ws1.json بصيغة {"dimensionCounts":{...},"units":[AuditUnit,...]}.
كل وحدة يجب أن تحمل: id (A-0001 تصاعديًا)، dimension، spec_id، old_ref، new_ref، method، verdict، severity، deviation، evidence، reviewer.
لا تضف وحدة بلا old_ref وnew_ref حقيقيين. الغرض: إعادة إثبات من الصفر، لا نسخ أحكام feature-parity.json.
```

- [ ] **Step 2: اكتب موجز الوكيل** — `.opencode/analyses/audit/prompts/ws1-brief.md`:

```text
# WS1 brief — behavior, data, compatibility

## نطاقك
- 181 وحدة سلوك F-001..F-181 من docs/spec/features.md (جدول المواصفات).
- 12 وحدة بيانات/تنسيقات من docs/spec/data-model.md: الجداول الخمسة، profile.json، instance.lock.json، operation journal، SQLite/WAL، وقراءة ملفات القديم في الجديد.
المجموع: 193 وحدة. لا تضف وحدات أخرى.

## method
لكل وحدة: افتح old_ref في D:\latest_project عند 4f71f18 وnew_ref في E:\ModManager عند HEAD. قارن الدلالة لا الأسماء.
- method=code إن كان الحكم بالقراءة.
- method=test إن كان هناك اختبار يحمل اسم spec_fNNN أو ما يعادله — اذكره في evidence.
- method=run إن شغّلت أمرًا (مثل cargo run -p mm-cli -- ...) — اذكر الأمر ونتيجته.
لا تستخدم browser ولا registry.

## verdicts
match | documented-deviation (إن كان Defer/Drop أو انحرافًا موثّقًا في feature-parity.json) | undocumented-deviation | unverifiable.
severity: blocker | high | medium | low | info. أي undocumented-deviation يجب أن يكون blocker/high/medium.
 reviewer: "deepseek".

## قواعد خاصة
- F-125 جزئي موثّق ⇒ documented-deviation, severity=low, deviation يذكر: للقراءة فقط، بلا كتابة هدف ولا اختيار إصدار آخر.
- لا تفترض أن «الاختبار موجود» يعني «السلوك مطابق» — اقرأ الاختبار.
- إن لم تجد new_ref حقيقيًا، اكتب verdict=unverifiable واشرح في evidence; لا تخترع مسارًا.

## الإخراج
ws1.json واحد، بترتيب A-0001..A-0193، وdimensionCounts {"behavior":181,"data":12}.
```

- [ ] **Step 3: تحقّق من مخرَج الوكيل**

Run:
```bash
node -e "const j=require('./.opencode/analyses/audit/ws1.json');if(j.units.length!==193)throw new Error(j.units.length);for(const u of j.units)for(const k of ['old_ref','new_ref','method','verdict','severity','evidence'])if(!u[k])throw new Error(u.id+' missing '+k);console.log('ws1 ok:',j.units.length)"
```
Expected: `ws1 ok: 193`.

- [ ] **Step 4: اجمع**

Commit: `git add .opencode/analyses/audit/prompts/ws1-brief.md .opencode/analyses/audit/ws1.json && git commit -m "audit(ws1): re-verify 181 behaviors and 12 data contracts"`

---

### Task 4: WS2 — الواجهة وi18n والأمان والأداء

**Files:**
- Create: `.opencode/analyses/audit/prompts/ws2-brief.md`
- Create: `.opencode/analyses/audit/ws2.json`
- Create/Delete: `preview-mock.html` (مؤقت، يُحذف قبل الالتزام)

**Interfaces:**
- Consumes: `audit-inventory.json` (ui=26، i18n=4، safety=10، performance=7) = 47 وحدة.
- Produces: `ws2.json` + لقطات تحت `.opencode/analyses/audit/shots/`.

- [ ] **Step 0: جهّز محفّح المتصفّح** (قرار المستخدم 2026-11-21):

```bash
cd E:\ModManager
cp .opencode/analyses/browser-preview.html preview-mock.html
# شغّل pnpm dev في الخلفية إن لم يكن المنفذ 1420 مستمعًا؛ ثم افتح http://localhost:1420/preview-mock.html
```
ملف `preview-mock.html` **مؤقت** ويُحذف قبل الالتزام؛ المحفّح الدائم هو `.opencode/analyses/browser-preview.html`. إن احتاج WS2 بياناتًا إضافية (مثل `validate_instance`) يُضاف إلى المحفّح الدائم أولًا ثم يُنسخ.

- [ ] **Step 1: اكتب موجز الوكيل**:

```text
# WS2 brief — UI, i18n, safety, performance

## نطاقك (47 وحدة)
- 26 شاشة من docs/spec/screens.md §3 (24 classed + نافذتان بلا صنف: `Target Version Planner` و`Saved mods`) — قارن كل شاشة بالواجهة الحالية في src/app/pages/.
- 4 وحدات i18n: اكتمال en/ar، RTL، مفاتيح خامّة،_plural forms.
- 10 وحدات safety: حواجز المسار، SHA-1/SHA-512، .part ثم rename، الأرشيف enclosed_name، symlinks، الحدود، :disabled، fail-closed redaction، PRESENT مقابل ELIGIBLE، التقليم الجاف.
- 7 وحدات performance (`B-1..B-7`) من docs/spec/performance-budgets.md: throttle التقدّم، التمرير الافتراضي، الإلغاء، الذاكرة، زمن الفحص، حجم الحزمة.

## إلزام المتصفّح
لكل واجهة (26 + i18n): افتح http://localhost:1420/preview-mock.html في تبويب المتصفّح المدمج، ثم snapshot وconsole(level=error) وscreenshot.
- method=browser وevidence يجب أن يحتوي كلمة console ونتيجة الفحص (مثال: "console: 0 errors; raw keys: none").
- احفظ اللقطة في .opencode/analyses/audit/shots/<id>.jpeg واذكر مسارها في evidence.
- أي مفتاح خام أو خطأ console ⇒ undocumented-deviation بseverity high على الأقل.
- إن تعذّر فتح المتصفّح ⇒ verdict=unverifiable واشرح السبب؛ لا تدّعِ النجاح.

## البقية
safety/performance: method=code أو test أو run. اقرأ الكود الفعلي؛ لا تكتفِ باسم الاختبار.

## الإخراج
.opencode/analyses/audit/ws2.json بصيغة {"dimensionCounts":{"ui":26,"i18n":4,"safety":10,"performance":7},"units":[...]}، معرّفات A-0194..A-0240. reviewer="deepseek" للكود، "browser" للواجهات.
```

- [ ] **Step 2: شغّل الوكيل** — subagent `general` بنموذج deepseek يقرأ الموجز وينفّذه، مع تذكير: «لا تعديل على مصادر المشروع؛ المتصفّح للقراءة فقط».

- [ ] **Step 3: تحقّق**

Run:
```bash
node -e "const j=require('./.opencode/analyses/audit/ws2.json');if(j.units.length!==47)throw new Error(j.units.length);let b=0;for(const u of j.units)if(u.method==='browser'){if(!/console/i.test(u.evidence))throw new Error(u.id+' browser without console');b++};console.log('ws2 ok:',j.units.length,'browser:',b)"
```
Expected: `ws2 ok: 47 browser: 30`.

- [ ] **Step 4: احذف المحفّح المؤقت واجمع**

```bash
rm -f preview-mock.html
git add .opencode/analyses/audit/prompts/ws2-brief.md .opencode/analyses/audit/ws2.json .opencode/analyses/audit/shots
git commit -m "audit(ws2): verify UI in the embedded browser, plus safety and performance"
```
Expected: `git status` نظيف، ولا `preview-mock.html` في الالتزام.

---

### Task 5: WS3 — التغليف M4 والتبعيات وCLI

**Files:**
- Create: `.opencode/analyses/audit/prompts/ws3-brief.md`
- Create: `.opencode/analyses/audit/ws3.json`

**Interfaces:**
- Consumes: `audit-inventory.json` (packaging=8، deps=4، cli=6) = 18 وحدة.
- Produces: `ws3.json`، معرّفات A-0241..A-0258.

- [ ] **Step 1: اكتب موجز الوكيل**:

```text
# WS3 brief — packaging M4, dependencies, CLI/IPC

## نطاقك (18 وحدة)
- 8 تغليف: مثبتات Windows (MSI/NSIS) وAppImage/Linux، التوقيع، التحديث التلقائي، ترحيل إعدادات/نسخ القديم، قنوات الإصدار، حجم الحزمة، التشخيص، release manifest.
- 4 تبعيات: crates الأساسية (tokio/reqwest/rusqlite/zip/ts-rs)، حزم npm (React/Tauri/Tailwind)، التراخيص، تاريخ الصيانة.
- 6 CLI/IPC: أوامر mm-cli، أوامر Tauri، IpcError codes، plan-then-execute، dry-run، التقدّم عبر Channels.

## registry إلزامي
لكل تبعية: method=registry وevidence يجب أن يحتوي تاريخ أحدث إصدار والترخيص وحالة الصيانة. استخدم context7 أو صفحة الحزمة عبر webfetch. لا تخمن.

## قاعدة M4
M4 قرارٌ مفتوح في foundations.md وADR 0003 — لا تحكم بـundocumented-deviation لمجرد غياب التغليف؛ صنّفه `documented-deviation` بـ`severity=info` مع الإشارة إلى التعارض الموثّق.

## الإخراج
ws3.json بصيغة {"dimensionCounts":{"packaging":8,"deps":4,"cli":6},"units":[...]}.
```

- [ ] **Step 2: شغّل الوكيل** — subagent `general` بـ deepseek.

- [ ] **Step 3: تحقّق**

Run:
```bash
node -e "const j=require('./.opencode/analyses/audit/ws3.json');if(j.units.length!==18)throw new Error(j.units.length);for(const u of j.units.filter(x=>x.method==='registry'))if(!/license|ترخيص/i.test(u.evidence))throw new Error(u.id+' registry without license');console.log('ws3 ok:',j.units.length)"
```
Expected: `ws3 ok: 18`.

- [ ] **Step 4: اجمع**

Commit: `git add .opencode/analyses/audit/prompts/ws3-brief.md .opencode/analyses/audit/ws3.json && git commit -m "audit(ws3): compare packaging M4, dependencies and CLI/IPC"`

---

### Task 6: WS4 — الدروس والاختبارات

**Files:**
- Create: `.opencode/analyses/audit/prompts/ws4-brief.md`
- Create: `.opencode/analyses/audit/ws4.json`

**Interfaces:**
- Consumes: `audit-inventory.json` (lessons=10، tests=8) = 18 وحدة.
- Produces: `ws4.json`، معرّفات A-0259..A-0276.

- [ ] **Step 1: اكتب موجز الوكيل**:

```text
# WS4 brief — lessons and test coverage

## نطاقك (18 وحدة)
- 10 دروس من docs/spec/lessons.md §1..§10. أهمّها §4.2 (خلط PRESENT/ELIGIBLE أدى إلى حذف بيانات في القديم): تحقّق أن الجديد يفصل المجموعتين فعليًا وباختبار يحمل spec_f131_*.
- 8 وحدات اختبارات: قارن 34 ملف test_*.py قديمًا بـ24 ملف اختبار Rust + 5 TS في الجديد. ركّز على: التنزيل، المزوّدون، السلامة، DB، journal، resolver، mrpack، logging، inventory، i18n.
لكل درس: method=code أو test، وevidence يذكر ملف/اختبارًا حقيقيًا. إذا كان الدرس «تعلّمه القديم بالطريقة الصعبة» وفُعِل في الجديد ⇒ match. إذا عاد الخطأ ⇒ undocumented-deviation severity=blocker.
لا تفترض أن عدد الاختبارات الأكبر يعني تغطية أكبر — اقرأ أسماء الاختبارات وما تختبره.

## الإخراج
ws4.json بصيغة {"dimensionCounts":{"lessons":10,"tests":8},"units":[...]}.
```

- [ ] **Step 2: شغّل الوكيل** — subagent `general` بـ deepseek.

- [ ] **Step 3: تحقّق**

Run:
```bash
node -e "const j=require('./.opencode/analyses/audit/ws4.json');if(j.units.length!==18)throw new Error(j.units.length);const l=j.units.find(u=>u.dimension==='lessons'&&/4\.2|PRESENT/.test(u.evidence||''));if(!l)throw new Error('missing the §4.2 deletion-bug check');console.log('ws4 ok:',j.units.length)"
```
Expected: `ws4 ok: 18`.

- [ ] **Step 4: اجمع**

Commit: `git add .opencode/analyses/audit/prompts/ws4-brief.md .opencode/analyses/audit/ws4.json && git commit -m "audit(ws4): re-check old lessons and ported test coverage"`

---

### Task 7: دمج المسارات في الدفتر

**Files:**
- Modify: `docs/spec/audit-ledger.json`
- Modify: `docs/spec/audit-findings.md` (مولّد)

**Interfaces:**
- Consumes: `ws1.json`..`ws4.json` (276 وحدة) + Task 1 schema.
- Produces: دفترٌ كامل بـ276 وحدة، ومخرَج مُعاد توليده.

- [ ] **Step 1: ادمج**

Run:
```bash
node -e "
const fs=require('fs');const cp=require('child_process');
const parts=['ws1','ws2','ws3','ws4'].map(n=>JSON.parse(fs.readFileSync('.opencode/analyses/audit/'+n+'.json','utf8')).units);
const units=parts.flat();
if(units.length!==276)throw new Error('expected 276 units, got '+units.length);
const ids=units.map(u=>u.id);if(new Set(ids).size!==ids.length)throw new Error('duplicate ids');
for(let i=0;i<units.length;i++)if(units[i].id!=='A-'+String(i+1).padStart(4,'0'))throw new Error('bad id order at '+i+' '+units[i].id);
const ledger={version:1,baseCommit:'4f71f18736c95a1eeb877fe8363f11871118c618',headCommit:cp.execSync('git rev-parse HEAD',{encoding:'utf8'}).trim(),units};
fs.writeFileSync('docs/spec/audit-ledger.json',JSON.stringify(ledger,null,2));
console.log('merged:',units.length);
"
```
Expected: `merged: 276`.

- [ ] **Step 2: ولّد وتحقّق**

Run: `pnpm audit:report && pnpm audit:check`
Expected: `audit: wrote ... (276 units)` ثم `audit: findings up to date`.

- [ ] **Step 3: اجمع**

Commit: `git add docs/spec/audit-ledger.json docs/spec/audit-findings.md && git commit -m "audit: merge four workstreams into the 276-unit ledger"`

---

### Task 8: المراجعة العدائية للـfindings

**Files:**
- Create: `.opencode/analyses/audit/audit-review.md`
- Modify: `docs/spec/audit-ledger.json` (تصحيح findings مرفوضة فقط)

**Interfaces:**
- Consumes: `audit-findings.md` + الدفتر.
- Produces: قرارٌ لكل `undocumented-deviation`: `confirmed` أو `rejected` مع سبب؛ وحقل `reviewNote` يُضاف للوحدة.

- [ ] **Step 1: اكتب سؤال المراجعة** — `.opencode/analyses/audit/audit-review.md`:

```text
راجع عدائيًّا findings التدقيق في E:\ModManager (دفتر docs/spec/audit-ledger.json، تقرير docs/spec/audit-findings.md). للقراءة فقط: لا تعدّل الدفتر.
لكل undocumented-deviation وكل blocker:
1) افتح old_ref في D:\latest_project @ 4f71f18 وnew_ref في E:\ModManager @ HEAD وتحقّق أن الانحراف حقيقي.
2) تحقّق أن method مناسبة وأن evidence نتيجةٌ فعلية لا تخمين.
3) تحقّق أن severity مبرّرة: blocker فقط لفقدان بيانات/كسر أمان.
4) إن كان الانحراف موثّقًا أصلًا في feature-parity.json أو decision-sheet.md أو lessons.md ⇒ أوصِ بـdocumented-deviation.
5) إن كان method=browser ⇒ تحقّق من وجود اللقطة ومن سجل console المذكور.
أخرج جدولًا: id | الحكم (confirmed/rejected/downgrade) | السبب | التعديل المطلوب على الحقل.
لا مجاملات؛ اذكر file::line.
```

- [ ] **Step 2: شغّل مراجعًا مستقلًّا** — subagent `general` بـ deepseek، جلسة جديدة، يقرأ السؤال وينفّذه.

- [ ] **Step 3: طبّق التعديلات المقبولة فقط**

Run (يُعدَّل حقل `reviewNote` ويُعاد التوليد):
```bash
node -e "
const fs=require('fs');const p='docs/spec/audit-ledger.json';const j=JSON.parse(fs.readFileSync(p,'utf8'));
// مثال: j.units.find(u=>u.id==='A-0007').reviewNote='confirmed by adversarial reviewer';
fs.writeFileSync(p,JSON.stringify(j,null,2));
"
pnpm audit:report && pnpm audit:check
```
Expected: `audit: findings up to date`، وعدد الـfindings يتغيّر فقط وفق recommendations المقبولة.

- [ ] **Step 4: اجمع**

Commit: `git add .opencode/analyses/audit/audit-review.md docs/spec/audit-ledger.json docs/spec/audit-findings.md && git commit -m "audit: adversarial review of findings"`

---

### Task 9: التقرير الختامي وبوابة CI

**Files:**
- Create: `docs/spec/audit-report.md`
- Modify: `.github/workflows/ci.yml`

**Interfaces:**
- Consumes: `audit-findings.md` النهائي + `audit-review.md`.
- Produces: تقريرٌ عربي + بوابة `pnpm audit:check` في CI.

- [ ] **Step 1: اكتب التقرير** — `docs/spec/audit-report.md` بالبنية التالية (بدون عناصر ناقصة):

```markdown
# التقرير الختامي للتدقيق الشامل — القديم ↔ الجديد

> BASE: `4f71f18` · HEAD: `<git rev-parse HEAD>` · الدفتر: 276 وحدة.

## ما تحقّقتُ منه
(جدول الأبعاد: عدد الوحدات، مطابق، انحراف موثّق، findings، غير مُتحقَّق — من audit-findings.md)

## Findings مرتّبة بالخطورة
(لكل finding: id · الأثر · old_ref · new_ref · method · severity · التوصية · الشريحة المقترحة)

## انحرافات موثّقة
(F-125 · Defer · Drop · M4 — لماذا لا تُعدّ عيوبًا)

## ما لم أتحقق منه
(وحدات verdict=unverifiable + قيود المتصفّح/القديم)

## التوصية قبل الإطلاق
(قائمة بـ blocker/high مع «يُوصى بالإصلاح قبل الإصدار» أو «يُقبل مع توثيق»)
```

- [ ] **Step 2: أضف خطوة CI** — في `.github/workflows/ci.yml` بعد `pnpm feature-parity:check`:

```yaml
      - run: pnpm audit:check
```

- [ ] **Step 3: شغّل البوابات كلها**

Run:
```bash
pnpm audit:test && pnpm audit:check && pnpm parity:check && pnpm feature-parity:check && pnpm i18n:check && pnpm build
```
Expected: كلها تمرّ؛ لا أخطاء.

- [ ] **Step 4: اجمع وادفع**

Commit: `git add docs/spec/audit-report.md .github/workflows/ci.yml && git commit -m "audit: final report and CI gate"`
Run: `git push -u origin docs/audit-full-parity`

---

## Self-Review

- **Spec coverage:** §1 (إعادة إثبات من الصفر) ← Tasks 3–6 · §3 (الدفتر) ← Task 1+7 · §4 (المسارات) ← Tasks 3–6 · §5 (verdict/severity/مراجع عدائي) ← Task 1+8 · §6 (المخرجات/البوابات) ← Task 1+9 · §7 (الترتيب) ← 1→9 · §8 (خارج النطاق) ← Global Constraints · §9 (المخاطر) ← Task 8 + browser في Task 4. لا فجوة.
- **Placeholder scan:** لا `TBD`/`TODO`/«أضف معالجة مناسبة»؛ كل خطوة كود أو أمر أو موجز كامل.
- **Type consistency:** `AuditUnit` بنفس الحقول في Tasks 1/3–8؛ `A-NNNN` متسلسل 0001–0276 عبرTasks 3–7؛ `pnpm audit:check` بنفس الاسم في Task 1 وCI؛ `ws1..ws4.json` بنفس الأسماء في Tasks 3–7.

---

## Execution Handoff

الخطة محفوظة في `docs/superpowers/plans/2026-11-21-full-parity-audit.md`. خياران للتنفيذ:

1. **Subagent-Driven (موصى به)** — وكيلٌ جديد لكل مهمة، مراجعةٌ بين المهام.
2. **Inline Execution** — تنفيذ المهام في هذه الجلسة عبر executing-plans، بدفعات مع نقاط مراجعة.

ايّ اختيار؟
