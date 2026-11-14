#!/usr/bin/env node
/**
 * build-feature-parity.mjs — يبني «مصفوفة تكافؤ الميزات»: هل كلُّ سلوكٍ
 * كان في التطبيق القديم موجودٌ في نسخة الرست؟
 *
 * ثلاثة مصادرُ أدلّة، ولكلٍّ درجتُه:
 *   1. مراجعةٌ مُتحقَّقة  (docs/spec/feature-parity.json) — حكمٌ على السلوك
 *      بأدلّةٍ ملموسة، مراجَعٌ مرّةً ثانية من وكيلٍ مستقلّ.
 *   2. تتبّعُ اختبارٍ     (docs/spec/parity.md) — اختبارٌ يحمل معرّف المواصفة.
 *   3. قرارُ المواصفة     (docs/spec/features.md) — `Defer`/`Drop`.
 *
 * الفجوةُ في الأدلّة هي التي تُبلَّغ: كلُّ بندٍ `Keep` يجبُ أن يكون له حكمٌ
 * مُراجَعٌ أو تتبّعُ اختبار؛ وإلا فالبندُ «قيد المراجعة» ويُفشل الفحص.
 *
 *   node scripts/build-feature-parity.mjs [--check]
 */
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const ROOT = new URL("..", import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, "$1");
const SPEC = join(ROOT, "docs/spec/features.md");
const PARITY = join(ROOT, "docs/spec/parity.md");
const REVIEWED = join(ROOT, "docs/spec/feature-parity.json");
const OUT = join(ROOT, "docs/spec/feature-parity.md");
const CHECK = process.argv.includes("--check");

const read = (p) => readFileSync(p, "utf8");
const json = (p) => JSON.parse(read(p));

/* 1) المواصفات */
const specRows = [];
for (const line of read(SPEC).split(/\r?\n/)) {
  if (!/^\|\s*\*\*F-\d{3}\*\*/.test(line)) continue;
  const c = line.split("|").slice(1, -1).map((x) => x.trim());
  const id = (c[0] || "").replace(/\*/g, "");
  if (!/^F-\d{3}$/.test(id)) continue;
  // القرارُ يُقرأ بالنمط لا بالعمود: للجداول بنسختين (بعضُ البنود في جدولٍ
  // «سلوكٍ محذوف» بأعمدةٍ مختلفة)، وعمودُ القرارِ فيهما ليس العمود نفسه.
  const dm = line.match(/\*\*(Keep|Defer|Drop)\*\*/);
  const decision = dm ? dm[1] : "";
  const domainCell = c[3] || "";
  const domain = /[`/\\]/.test(domainCell) ? "-" : domainCell;
  specRows.push({ id, name: c[1] || "", domain, decision, phase: c[6] || "" });
}

/* 2) تتبّعُ الاختبارات من مصفوفة التتبّع */
const traced = new Map(); // id -> evidence text
{
  const lines = read(PARITY).split(/\r?\n/);
  for (const line of lines) {
    const m = line.match(/^\|\s*\*\*(F-\d{3})\*\*\s*\|\s*(.+?)\s*\|\s*(.+?)\s*\|\s*(.+?)\s*\|\s*(.+?)\s*\|\s*(.+?)\s*\|$/);
    if (!m) continue;
    const [, id, , , , tier, evidence] = m;
    if (!tier || tier === "مؤجَّل" || tier === "متروك") continue;
    if (!tier.includes("مُختبَر") && !tier.includes("الكود")) continue;
    traced.set(id, evidence);
  }
}

/* 3) الأحكامُ المُراجَعة */
const reviewed = new Map();
for (const s of json(REVIEWED).specs) reviewed.set(s.id, s);

const STATUS_LABEL = {
  present: "حاضر — مُراجَع",
  partial: "جزئي — مُراجَع",
  missing: "مفقود — مُراجَع",
  "not-applicable": "خارج النطاق — مُراجَع",
};

const rows = [];
const missingReview = [];
for (const s of specRows) {
  const rev = reviewed.get(s.id);
  const tr = traced.get(s.id);
  let status, evidence, note, gap = null;
  if (s.decision === "Drop") {
    status = "مُستبعد (قرارُ المواصفة)";
    evidence = "—";
    note = "سلوكٌ حُذف بقرارٍ موثَّق.";
  } else if (s.decision === "Defer") {
    status = "مؤجَّل (قرارُ المواصفة)";
    evidence = "—";
    note = "سلوكٌ مؤجَّلٌ بقرارٍ موثَّق.";
  } else if (rev) {
    status = STATUS_LABEL[rev.status] || rev.status;
    evidence = rev.evidence.length ? rev.evidence.join(" · ") : "—";
    note = rev.note || "";
    gap = rev.gap || null;
  } else if (tr) {
    status = "حاضر — بتتبّعِ اختبار";
    evidence = tr;
    note = "الدليل: اختبارٌ يحمل المعرّف؛ لم تُراجَع دلالتُه يدويًّا بعد.";
  } else {
    status = "قيد المراجعة";
    evidence = "—";
    note = "لا حكمَ مُراجَعًا ولا تتبّعَ اختبار.";
    missingReview.push(s.id);
  }
  rows.push({ ...s, status, evidence, note, gap });
}

const keep = rows.filter((r) => r.decision === "Keep");
const count = (pred) => keep.filter(pred).length;
const byStatus = (label) => keep.filter((r) => r.status.startsWith(label)).length;

const L = [];
L.push("# مصفوفة تكافؤ الميزات — القديم ↔ نسخة الرست");
L.push("");
L.push("> **مولَّدة آليًّا**: `node scripts/build-feature-parity.mjs` من `docs/spec/features.md`");
L.push("> (سلوكُ التطبيق القديم) و`docs/spec/parity.md` (تتبّعُ الاختبارات) و`docs/spec/feature-parity.json`");
L.push("> (أحكامٌ مُراجَعةٌ بأدلّة، مراجَعٌ مرّةً ثانية من وكيلٍ مستقلّ).");
L.push("");
L.push("> **درجاتُ الدليل**: «مُراجَع» = حكمٌ على السلوك بأدلّةٍ ملموسة · «بتتبّعِ اختبار» = اختبارٌ يحمل");
L.push("> المعرّف (دليلٌ على وجود الشيفرة، لا على مطابقتها للسلوك) · «قرارُ المواصفة» = `Defer`/`Drop`.");
L.push("");
L.push("## الملخّص");
L.push("");
L.push(`- **${rows.length}** بندًا في المواصفات · **${keep.length}** منها \`Keep\` · ${rows.filter((r) => r.decision === "Defer").length} \`Defer\` · ${rows.filter((r) => r.decision === "Drop").length} \`Drop\`.`);
L.push("");
L.push("| الدرجة | بنود `Keep` |");
L.push("|---|---|");
L.push(`| حاضر — مُراجَع | ${byStatus("حاضر — مُراجَع")} |`);
L.push(`| حاضر — بتتبّعِ اختبار | ${byStatus("حاضر — بتتبّع")} |`);
L.push(`| جزئي — مُراجَع | ${byStatus("جزئي — مُراجَع")} |`);
L.push(`| مفقود — مُراجَع | ${byStatus("مفقود — مُراجَع")} |`);
L.push(`| خارج النطاق — مُراجَع | ${byStatus("خارج النطاق — مُراجَع")} |`);
L.push(`| قيد المراجعة (لا دليل) | ${byStatus("قيد المراجعة")} |`);
L.push("");
const gaps = keep.filter((r) => r.status.startsWith("مفقود") || r.status.startsWith("جزئي"));
L.push(`**${gaps.length}** بندًا ` + "`Keep`" + ` يحتاج عملًا في نسخة الرست (مفقودٌ أو جزئي).`);
L.push("");
if (missingReview.length) {
  L.push("> ⚠ بنودٌ بلا دليلٍ ولا حكم: " + missingReview.join(", "));
  L.push("");
}
L.push("## ما ينقص نسخة الرست — قائمة العمل");
L.push("");
L.push("| المعرّف | السلوك | المجال | الحالة | ما ينقص بالضبط |");
L.push("|---|---|---|---|---|");
for (const r of gaps) L.push(`| **${r.id}** | ${r.name} | ${r.domain} | ${r.status} | ${r.gap || r.note} |`);
L.push("");
L.push("## التفاصيل");
L.push("");
let domain = null;
for (const r of rows) {
  if (r.domain !== domain) {
    domain = r.domain;
    L.push(`### ${domain}`);
    L.push("");
    L.push("| المعرّف | السلوك | القرار | المرحلة | الحالة | الدليل |");
    L.push("|---|---|---|---|---|---|");
  }
  L.push(`| **${r.id}** | ${r.name} | ${r.decision} | ${r.phase} | ${r.status} | ${r.evidence} |`);
}
L.push("");
L.push("---");
L.push("");
L.push("**ملاحظةٌ عن الأسلوب:** حيث وُجد السلوكُ في نسخة الرست فهو مُعادٌ بأسلوبِ رست: أخطاءٌ مُصنَّفةٌ عبر IPC");
L.push("(`IpcError { code, message }`) بدلَ النصوصِ الحرّة، وخطةٌ قبل التنفيذ، وكتابةٌ ذرّيةٌ إلى `.part` مع فحصِ هاش");
L.push("قبل إعادة التسمية، ورشّحاتُ مسارٍ قبل الاستخراج، وقنواتُ تقدّمٍ بدلُ حجبِ الواجهة. أمّا ما هو مفقودٌ فيبقى مفقودًا.");
L.push("");

const text = L.join("\n");
if (CHECK) {
  let current = "";
  try { current = read(OUT); } catch { /* لا ملف بعد */ }
  if (current !== text) {
    console.error("feature-parity: out of date — run `node scripts/build-feature-parity.mjs`");
    process.exit(1);
  }
  if (missingReview.length) {
    console.error(`feature-parity: ${missingReview.length} spec(s) have no verdict and no trace`);
    process.exit(1);
  }
  console.log("feature-parity: up to date");
} else {
  writeFileSync(OUT, text, "utf8");
  console.log(
    `feature-parity: wrote docs/spec/feature-parity.md — specs=${rows.length} keep=${keep.length} ` +
      `reviewed=${byStatus("حاضر — مُراجَع") + byStatus("جزئي — مُراجَع") + byStatus("مفقود — مُراجَع") + byStatus("خارج النطاق — مُراجَع")} ` +
      `traced=${byStatus("حاضر — بتتبّع")} gaps=${gaps.length} unreviewed=${missingReview.length}`
  );
}
