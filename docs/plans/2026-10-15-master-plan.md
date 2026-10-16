# ModManager — الخطة الرئيسية للتنفيذ المتتابع

> **For agentic workers:** REQUIRED SUB-SKILL: استخدم `subagent-driven-development` (موصى به) أو `executing-plans` لتنفيذ هذه الخطة مهمةً مهمة. الخطوات بصيغة `- [ ]` للtracking. كل مهمة تنتهي ببوابة مراجعة إلزامية (§بروتوكول المراجعة).

**Goal:** تسليم مدير مودات ومود باكات لويندوز ولينكس مبني على Tauri 2 + Rust + React، مستخرج السلوك من تطبيق Python قديم مرجعياً فقط، عبر ثماني مراحل متتابعة تفصل بينها بوابات قرار بشرية.

**Architecture:** مكتبة `core/` لا تعرف Tauri ولا الواجهة؛ تطبيق Tauri وأداة CLI عميلان رفيعان لها. الأنواع تعبر IPC عبر `ts-rs` (مولَّدة، مُلتزَمة). كل تعديل: خطة أولاً ثم تنفيذ، وكل تنزيل: `.part` ثم تحقّق بصمة ثم إعادة تسمية ذرّية.

**Tech Stack:** Tauri 2 · Rust (tokio, reqwest/rustls, rusqlite, zip, sha1/sha2, thiserror, tracing) · React 19 + TypeScript strict + Vite · Tailwind CSS 4 + shadcn/ui · TanStack Query/Virtual · Zustand · Wouter · i18next (RTL) · Biome · ts-rs · pnpm

## Global Constraints

هذه تخصّ **كل** مهمة ضمنياً ولا تُكرَر:

- الكود والتعليقات والمعرّفات وأسماء الاختبارات ورسائل الـcommit: **إنجليزية**. النصوص والمقارير: **عربية**.
- `core/` **لا يعتمد** على Tauri ولا على أي واجهة. الـwebview لا يمسّ نظام الملفات ولا الشبكة ولا SQL.
- أخطاء IPC منمّطة: `IpcError { code, message }`. الواجهة تترجم `code` لا النص.
- **بلا `unwrap()` ولا `expect()`** في شيفرة الإنتاج. مسموح في الاختبارات.
- **بلا نصوص UI ثابتة** — مفاتيح i18n من أول شاشة. التخطيط يعمل في RTL.
- كل تعديل: خطة (adds/updates/removals) تُعرض ثم تُطبَّق. يدعم dry-run.
- كل تنزيل: `.part` ← تحقّق SHA-1/SHA-512 ← إعادة تسمية ذرّية.
- فكّ الضغط: `enclosed_name()`، رفض المسارات المطلقة و`..` والـsymlinks، وحدود للحجم الكلي وحجم الملف وعدد الملفات.
- التقدّم عبر Channels، مقيَّد إلى 100–250ms.
- SQLite: WAL، ترقيم migrations، نسخة احتياطية قبل الترحيل، رفض فتح مخطّط أحدث من المدعوم.
- اختبار الملفات والمجلدات المؤقتة: **دلائل مؤقتة فقط**، وعلى **NTFS** لأن الـhard links تحتاجه. **لا تلمس مجلدات مودات حقيقية أبداً.**
- تبعيات جديدة: افحص صفحة السجلّ (آخر تاريخ إصدار، عدد التحميلات، الرخصة، الصيانة) **قبل** الإضافة. أفضّل std أو ~100 سطر من كودنا على crate مجهول صغير.
- **لا تخترع API shapes.** راجع توثيق Modrinth / CurseForge الرسمي. إن تعذّر: **اسأل**.
- كل مهمة تنتهي بأوامر «تعريف الإنجاز» الأربعة موثّقة بمخرجاتها الحقيقية.

## تعريف الإنجاز (Definition of Done) — يُشغَّل في نهاية كل مهمة

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace          # يولّد أيضاً src/bindings — تُلتزَم
pnpm build                      # فحص أنواع الواجهة
```

**لا تُعتبر المهمة منجزة قبل عرض المخرجات الحقيقية.** أي أمر لا يعمل: اذكر سبباً محدّداً، ولا تُسَمّ النتيجة «ناجحة».

---

## خريطة الملفات — تُثبَّت قبل Task 3

الملكية ومسؤولية كل ملف تُثبَت هنا ولا تُناقَض:

| المسار | المسؤولية | يُنشأ في |
|---|---|---|
| `core/src/safety/` | التحقّق من المسار + فكّ الضغط الآمن + الكتابة الذرّية | T3 |
| `core/src/downloader/` | الطابور، التقييد، إعادة المحاولة، البصمة، الإلغاء | T3 |
| `core/src/db/` | rusqlite، WAL، migrations، api_cache، البوابة | T3 |
| `core/src/paths.rs` | مجلدات platform عبر `directories` | T3 |
| `core/src/providers/` | `Provider` trait + Modrinth + (CurseForge في T7) | T4 / T7 |
| `core/src/resolver/` | plan-first dependency resolution | T4 |
| `core/src/instances/` | العزل + `instance.lock.json` | T4 |
| `core/src/jar_reader/` | `fabric.mod.json`، `mods.toml`، كشف المحمّل، كاش الفحص | T4 |
| `core/src/modpack/` | `.mrpack` + حِزَم CurseForge عبر قواعد السلامة | T7 |
| `core/src/logging/` | tracing + ملف دوّار + منقّي أسرار | T3 |
| `src-tauri/src/commands.rs` | أوامر IPC وDTO فقط — **لا منطق أعمال** | T2/T3… |
| `src-tauri/src/dto.rs` | أنواع المشتقّة بـ`#[ts(export)]` | T2… |
| `src/lib/ipc.ts` | **الحدّ الوحيد** لنداء Rust من الواجهة | T2 |
| `src/app/` · `src/features/` · `src/components/` | الواجهة، بلا منطق أعمال | T5/T6 |
| `src/bindings/` | **مولَّدة — تُلتزَم.** أي انحراف يُفشِل CI | T2 |

---

## بروتوكول المراجعة — إلزامي بعد كل مهمة

يُنفَّذ **بعد** كل Task من T2 إلى T8 (هذا هو Prompt 9 الموحَّد):

1. راجع كل تغيير منذ `BASE_REF` مقابل §5 و§6 في `AGENTS.md`. لكل مخالفة: `file:line` + خطورة (Blocker / Major / Minor).
2. شغّل أوامر تعريف الإنجاز الأربعة. **أخرج المخرجات الحقيقية**، لا الحفظ.
3. افحص أمنياً بتوسّع: فكّ الضغط، التنزيلات، الكتابة على القرص، صلاحيات IPC، CSP.
4. تحقّق أن الاختبارات المسمّاة بمعرّفات الـspec **تُثبت** السلوك الموصوف فعلاً، وضع علامة على أي تأكيد أعرض من اللازم.
5. افحص كل تبعية جديدة: صفحة السجلّ، آخر إصدار، التحميلات، الرخصة، الضرورية.
6. **اكتب قائمة بما تعذّر التحقّق منه في التقرير السابق** — لا تسقطه.

المخرجات بالعربية: ملخّص · جدول المخالفات · مُتحقَّق مقابل غير مُتحقَّق · توصية go / no-go.

---

## حالة التنفيذ الحالية (2026-10-15)

| | |
|---|---|
| المستودع | `E:\ModManager` · HEAD `deaab4d` · **6 commits** · **80 ملفاً متتبَّعاً** · شجرة نظيفة |
| Spec | `docs/spec/` = 8 ملفات + `behaviors/` 15 ملفاً = **23 وثيقة** |
| ADR | `0001` `0002` `0003` |
| Task 0 | ✅ مكتمل (مع ملاحظة SDK أدناه) |
| Task 1 | ✅ مكتمل — 181 ميزة، 152 Keep / 16 Defer / 13 Drop |
| Task 2 | ✅ **مكتمل 2026-10-16** — البناء والاختبارات والقياسات والـcommit (`39b39bb`) |

⛔ **الحاجز السابق (حُلّ 2026-10-16):** كانت مكتبات Windows SDK مفقودة ⇒ `LNK1181`. **الحل:**
المالك حرّر `C:` (0.24 ← 5.74 GB)، وثُبّت SDK (`DesktopCPPx64` فقط، 1.7 GB) من الحزمة
المُنزَّلة أصلاً — بلا تحميل شبكة. البناء يعمل بلا `vcvars`.

**مُتحقَّق فعلاً:** `cargo fmt --all -- --check` exit 0 · `cargo check -p mm-core` exit 0 · `pnpm install` 32.9s / 83.2 MB / 669 ملفاً.
**غير مُتحقَّق:** زمن أول بناء كامل · حجم `target/` النهائي · كل ما عدا `mm-core`.

---

### Task 0: فحص البيئة — ✅ مكتمل

**Files:** لا شيء في المستودع. **Skills:** `verification-before-completion`.

**ما تم:** rustc/cargo 1.99.0 · rustup 1.29.1 · rustfmt 1.10.0 · clippy 0.1.99 · node 22.22.3 · pnpm 12.11.2 · git 2.54.0 · WebView2 155.0.4283.45 · MSVC VC.Tools 14.44.35207 على `E:\VSBuildTools`.

- [x] الخطوة 1: الإصدارات مثبتة وموثّقة
- [x] الخطوة 2: تثبيت MSVC بموافقة المالك
- [x] الخطوة 3: `rustc --version` · `cargo --version` · `node --version` · `pnpm --version` — الأربعة تنجح

**⛔ فجوة مفتوحة:** `vswhere` يدّعي `Windows11SDK.26100` مثبّتاً لكن `kernel32.lib` و`windows.h` **غير موجودين**، و`HKLM\...\SxS\VS7` غائب. أثر فشل تثبيت `1603` السابق. **يُعالَج في Task 2 خطوة 1.**

---

### Task 1: استخراج المواصفات — ✅ مكتمل

**Files:** `docs/spec/*` · `docs/adr/0003-scope-decisions.md` · `docs/foundations.md` (منسوخ، غير معدّل)

**قواعد اتّبعت:** `D:\latest_project` **قراءة فقط، لم يُمسّ**. لا نسخ شيفرة. الملفات الضخمة استُخرجت بنيتها من أسماء الأصناف والإجراءات لا سطراً سطراً. كل غير مؤكَّد مميّز. كل وثيقة < 400 سطر بالعربية ومعرّفات إنجليزية.

**المخرجات:** 181 صفاً في `features.md` بعمودَي القرار والمرحلة · 23 وثيقة سلوكية · `decision-sheet.md` للأسئلة المعلّقة · مقاييس أداء مقيسة من `docs/baseline.md` للتطبيق القديم.

**Skills:** `codebase-design` · `domain-modeling` (لصياغة ADR 0003).

---

### Task 2: M0 — الأساس 🚧

**BASE_REF:** `deaab4d`

**Files:**
- مُنجَز: `Cargo.toml` · `.cargo/config.toml` · `.gitignore` · `core/` · `cli/` · `src-tauri/` · `src/` · `.github/workflows/{ci,bundle}.yml` · `docs/adr/0001` `0002`
- مطلوب: `src/bindings/*` (مولَّدة) · `docs/baseline-new.md`

**Interfaces — يُنتجها:**
- `mm_core::AppInfo { name, version, os, arch }` — أُنشئت
- `mm_core::demo::Progress::{ Started{total}, Advanced{downloaded}, Finished }` — أُنشئت
- `modmanager_lib::commands::{ app_info, start_demo_download }` — أُنشئت
- `modmanager_lib::dto::{ AppInfoDto, DownloadEvent, IpcError }` — أُنشئت
- `src/lib/ipc.ts` — الحدّ الوحيد لنداء Rust

**Skills:** `writing-plans` (بلا) · `systematic-debugging` عند الفشل · `verification-before-completion` للإغلاق.

**الخطوات المتبقّية:**

- [ ] **الخطوة 1 — إصلاح البيئة (مالك فقط).** تثبيت Windows SDK. **شرط تحقّق قبل أي خطوة أخرى:** `dir /s /b "C:\Program Files (x86)\Windows Kits\10\kernel32.lib"` يُرجع نتيجة. **لا حلول بديلة.**
- [ ] **الخطوة 2 — أول بناء، مقيساً.** `cargo build --workspace` موقوتاً. سجّل: الزمن · حجم `target/` بالميغابايت · عدد الملفات · المساحة الحرّة على `E:`.
- [ ] **الخطوة 3 — توليد الـbindings.** `cargo test --workspace`. تحقّق أن `src/bindings/DownloadEvent.ts` مطابق تماماً لما يتوقّعه `src/App.tsx`. **ثبّت البصمة الدائمة للملفات المولَّدة في `src/bindings/.gitattributes`.**
- [ ] **الخطوة 4 — واجهة Tailwind 4 + shadcn.** `@tailwindcss/vite` · alias `@` · `components.json` · الوضع الداكن والفاتح · شاشة تجريبية دنيا. **بلا منطق أعمال.**
- [ ] **الخطوة 5 — القياس.** على هذا الجهاز، بارداً ودافئاً، 3 عمليات دافئة ثم الوسيط: زمن أول نافذة · زمن حتى الشاشة التفاعلية · ذاكرة خامل (`WorkingSet` + `PrivateBytes` على **كل** العمليات بما فيها WebView2). احفظ في `docs/baseline-new.md`.
- [ ] **الخطوة 6 — CI يتأكّد.** على Windows وUbuntu: `fmt` · `clippy` · `test` · فحص انحراف `src/bindings` · `pnpm build`.
- [ ] **الخطوة 7 — أوامر تعريف الإنجاز الأربعة.** أخرج المخرجات الحقيقية.
- [ ] **الخطوة 8 — تحقّق بشري.** `pnpm tauri dev` يعرض الشاشة وشريط التقدّم يتحرّك · `cargo run -p mm-cli -- info` يعمل.
- [ ] **الخطوة 9 — Commit.** Conventional Commits، إنجليزي.
- [ ] **الخطوة 10 — بروتوكول المراجعة** على `deaab4d..HEAD`.

**Acceptance:** أوامر التعريف الأربعة تنجح · الواجهة تُبنى · القياسات مسجّلة في `docs/baseline-new.md` · المراجعة توصي go.

---

### Task 3: M1أ — أساسيات النواة

**BASE_REF:** tag `m0`.

**Files (تُنشأ):**
`core/src/safety/{mod.rs,path.rs,extract.rs,atomic.rs}` · `core/src/downloader/{mod.rs,queue.rs,retry.rs,verify.rs}` · `core/src/db/{mod.rs,migrations.rs,cache.rs}` · `core/src/logging/{mod.rs,sanitize.rs}` · `core/src/paths.rs`

**Interfaces — تُعرَّف هنا وتُلتزم:**
```rust
// core/src/safety/path.rs
pub enum PathError { Absolute, Traversal, Symlink, Outside { base: PathBuf, candidate: PathBuf } }
pub fn enclosed(root: &Path, rel: &Path) -> Result<PathBuf, PathError>;

// core/src/safety/extract.rs
pub struct ExtractLimits { pub max_total: u64, pub max_file: u64, pub max_files: usize }
pub fn extract_safely(zip: &Path, dest: &Path, limits: ExtractLimits) -> Result<ExtractReport, ExtractError>;

// core/src/safety/atomic.rs
pub fn write_atomic(dest: &Path, bytes: &[u8]) -> Result<(), io::Error>;   // .part ثم rename

// core/src/downloader/mod.rs
pub trait ProgressSink: Send + Sync { fn on_progress(&self, p: Progress); }
pub enum Progress { Started { total: u64 }, Advanced { downloaded: u64 }, Finished }
pub enum Verify { Sha1, Sha512 }

// core/src/logging/sanitize.rs
pub fn sanitize(input: &str) -> String;   // يُخفي مفاتيح CurseForge والرموز
```

**Skills:** `test-driven-development` · `rust-best-practices` · `rust-async-patterns` · `verification-before-completion`.

**الخطوات (نمط ثابت لكل وحدة — كرّره):**
- [ ] اختبار فاشل باسم يحمل معرّف الـspec، مثل `spec_f0xx_rejects_path_traversal`
- [ ] شغّله وتأكّد أنه **يفشل** لسبب صحيح
- [ ] أصغر تنفيذ يجعله ينجح
- [ ] شغّل الاختبارات وتأكّد أنها تنجح
- [ ] `cargo fmt` + `cargo clippy -- -D warnings`
- [ ] commit

**Triggers الإلزامية:**
- `safety`: مسارات مطلقة · `..` · symlink · تجاوز `max_total` · تجاوز `max_file` · تجاوز `max_files` · قفل ملفات ويندوز · مسارات >260 محرفاً · حساسية حالة الأحرف
- `downloader`: تقييد التوازي · exponential backoff + jitter · `Retry-After` · URL تنزيل فارغ · إعادة توجيه زائدة · فشل البصمة **لا يكتب** الملف النهائي · لا مساحة كافية · **الإلغاء يُنهي بلا ملف `.part` يتبقى**
- `db`: WAL مفعّل · ترقيم migrations · **نسخة احتياطية قبل الترحيل** · رفض مخطّط أحدث · `api_cache` بـTTL
- `logging`: المنقّي يحذف المفتاح ولا يترك البادئة

**Acceptance:** أوامر التعريف الأربعة تنجح · كل trigger أعلاه له اختبار · ADR قصير لكل قرار غير بديهي · المراجعة go.

---

### Task 4: M1ب — Modrinth والتبعات و CLI

**BASE_REF:** tag `m1a`.

**Files:** `core/src/providers/{mod.rs,modrinth.rs}` · `core/src/resolver/{mod.rs,graph.rs,plan.rs}` · `core/src/instances/{mod.rs,lock.rs}` · `core/src/jar_reader/{mod.rs,loader.rs,cache.rs}` · `cli/src/{main.rs,commands/*}`

**Interfaces:**
```rust
// core/src/providers/mod.rs
#[async_trait] pub trait Provider: Send + Sync {
    async fn search(&self, q: &SearchQuery) -> Result<Vec<Project>, ProviderError>;
    async fn get_project(&self, id: &ProjectId) -> Result<Project, ProviderError>;
    async fn get_versions(&self, id: &ProjectId) -> Result<Vec<Version>, ProviderError>;
    async fn get_versions_by_hashes(&self, hashes: &[Hash]) -> Result<Vec<(Hash, Version)>, ProviderError>;
}

// core/src/resolver/plan.rs
pub enum PlanOp { Add(ProjectId, VersionId), Update(ProjectId, VersionId), Remove(ProjectId) }
pub struct Plan { pub ops: Vec<PlanOp>, pub warnings: Vec<String> }
pub fn resolve(req: &ResolveRequest, catalog: &Catalog) -> Result<Plan, ResolveError>;   // supports dry_run
```

**⛔ قاعدة صارمة:** **لا تخترع أشكال الـAPI.** راجع `docs/spec/behaviors/providers.md` ثم توثيق Modrinth الرسمي لـAPI v2 **قبل** كتابة الـclient. كل استجابة تُحوَّل إلى نماذج `core` — **لا يتسرّب شكل خاص بمزوّد**.

**Triggers الإلزامية:** فلترة بإصدار Minecraft ومحمّل (Fabric/Forge/NeoForge/Quilt) · release channel · تبعية required/optional/incompatible/embedded · **كشف الدورات** · كشف التعارض · `instance.lock.json` بإصدارات وبصمات · `fabric.mod.json` و`META-INF/mods.toml` و`neoforge.mods.toml` · **حالة التعارض الكاذب المعروفة من الـspec** · كاش `(path, size, mtime)` فلا يُفتح jar غير متغيّر · الفحص على خيط خلفي.

**Acceptance:** الاستجابات المسجَّلة fixtures — **لا شبكة حيّة في الاختبارات** · أوامر CLI: `search install update remove list scan plan` · من الـCLI أركّب أحدّث وأحذف مجموعة مودات حقيقية في **instance مؤقّت** على ويندوز · **اذكر لي الأوامر بالضبط لأتحقّق بنفسي** · المراجعة go.

---

### Task 5: M2أ — نظام التصميم وهيكل التطبيق

**BASE_REF:** tag `m1`.

**⛔ بوابة بشرية قبل أي تصميم — لا تتجاوزها:**
- [ ] **اسألني بالعربية** عن: 2–3 تطبيقات يعجبني شكلها · الكثافة المفضّلة (مضغوطة/مريحة) · تفضيل الداكن/الفاتح · **وانتظر إجابتي**
- [ ] اعرض **اتجاهين بصريين مختلفين** كصفحتَي HTML ساكنتين أفتحهما في المتصفّح (بلا Tauri): الطباعة · لوحة الألوان · المسافات · شاشة Instances نموذجية · صفّ قائمة مودات نموذجي
- [ ] **وانتظر اختياري**

**Files:** `src/app/{shell,nav,header,palette,theme}.tsx` · `src/components/ui/*` · `src/lib/{i18n.ts,dir.ts}` · `src/styles/theme.css`

**Skills:** `frontend-design` · `shadcn` · `brainstorming` (للتصميم).

**الخطوات:** `@theme` tokens لـTailwind 4 · ثيم داكن وفاتح · مكوّنات shadcn مربوطة بالـtokens · cmdk · مبدّل ثيم · i18next عربي(RTL)+إنجليزي · Wouter · أنماط loading/empty/error.

**Acceptance:** أوامر التعريف · الهيكل يعمل داكن/فاتع وRTL/LTR · **لقطات شاشة لكل حالة** للمراجعة · المراجعة go.

---

### Task 6: M2ب — الشاشات

**BASE_REF:** tag `m2a`. الترتيب: **Instances ← تفاصيل instance ← تصفّح/بحث ← قائمة التنزيلات ← الإعدادات.**

**قواعد:** الواجهة لا تمسّ ملفات ولا شبكة ولا SQL — كل شيء عبر `src/lib/ipc.ts`. قوائم 1000+ صف remain سلسة (TanStack Virtual). أحداث التقدّم **لا تُغرق** React. كل شاشة لها loading/empty/error. تنقّل بلوحة المفاتيح. **بلا نصوص ثابتة.**

**Skills:** `frontend-design` · `shadcn` · `requesting-code-review`.

- [ ] قياس ومقارنة: زمن عرض القائمة · زمن الاستجابة للبحث · التجمّد >200ms أثناء التنزيل · الذاكرة — **احفظ في `docs/baseline-new.md`**
- [ ] **جدول المقارنة (القديم مقابل الجديد) في التقرير. لا تعلن نجاحاً بلا مقارنة.**

**Acceptance:** أوامر التعريف · جدول المقارنة في التقرير · المراجعة go.

---

### Task 7: M3 — CurseForge والمود باكات والمودات اليدوية

**BASE_REF:** tag `m2`.

**Files:** `core/src/providers/curseforge.rs` · `core/src/modpack/{mod.rs,mrpack.rs,curseforge.rs}` · `core/src/adopt.rs`

- [ ] مزوّد CurseForge (API v1): المفتاح من إعداد المستخدم أو البيئة — **لا يُلتزَم أبداً** · `fingerprints` بـMurmurHash2 بعد تجريد الفراغات — **تحقّق من التوثيق الرسمي**
- [ ] حالة **غياب رابط تنزيل**: افتح صفحة الملف في المتصفّج وأرشد المستخدم
- [ ] **اقترح في ADR**هل نحتاج proxy صغير للمفتاح — **ولا تبنِه بلا موافقتي**
- [ ] `.mrpack` (`modrinth.index.json` + `overrides`) وحِزَم CurseForge (`manifest.json` + `overrides`) عبر قواعد السلامة، مع تقدّم ومعاينة خطة
- [ ] تبنّي المودات اليدوية بالتعرّف على البصمة
- [ ] تصدير/استيراد `instance.lock.json`
- [ ] واجهة لما سبق وفق نظام التصميم القائم

**Acceptance:** أوامر التعريف · استيراد `.mrpack` حقيقي وحزمة CurseForge حقيقية في **instance مؤقّت** من طرف إلى طرف · fixtures الحقيقية من `docs/spec/fixtures.md` · **اذكر لي طريقة التحقّق بالضبط** · المراجعة go.

---

### Task 8: M4 — الإصدار

**BASE_REF:** tag `m3`.

- [ ] تحديث ذاتي بـ`tauri-plugin-updater`: **اشرح خطوة إعداد مفتاح التوقيع وموضع حفظ المفتاح الخاص — ولا تولّد مفتاحاً ولا تُلتزم واحداً بلا موافقتي الصريحة**
- [ ] مُثبِّتات: Windows (NSIS أو MSI) وLinux (AppImage, deb, rpm) · وثّق خيارات توقيع ويندوز وFlatpak كخطوة لاحقة
- [ ] `release.yml`: بناء على Windows وUbuntu، إرفاق المُثبِّتات، نشر بيانات التحديث
- [ ] i18n كامل · تقارير انهيار اختيارية (**معطّلة افتراضياً**) · شاشة About/Logs بزر «نسخ التشخيص» **يُنقّي الأسرار**
- [ ] توثيق المستخدم: التثبيت · أول تشغيل · الاستيراد من التطبيق القديم — **فقط إن كان قرار الترحيل في `docs/foundations.md` §14 قد مُنِجَزاً**

**Acceptance:** مُثبِّتات CI تُثبَّت وتعمل على ويندوز ولينكس · تحديث من N إلى N+1 يعمل · أوامر التعريف · المراجعة go.

---

## بوابات القرار البشرية — لا تتجاوزها آلياً

| # | البوابة | أين |
|---|---|---|
| 1 | تثبيت Windows SDK | T2 خطوة 1 |
| 2 | Keep/Drop/Defer لـ181 ميزة | **مُنجَز** — `decision-sheet.md` |
| 3 | اختيار الاتجاه البصري | T5 قبل التصميم |
| 4 | الموافقة على proxy مفتاح CurseForge | T7 |
| 5 | الموافقة على توليد مفتاح توقيع | T8 |
| 6 | go / no-go للدمج | بعد كل مهمة |

## Self-Review — ما لم تُغطَّ هذه الخطة

1. **قاعدة «كود كامل في كل خطوة» من `writing-plans` لم تتحقّق بالكامل** في T3–T8. السبب: `AGENTS.md` §6 يمنع اختراع أشكال الـAPI، وثلاث مراحل (T5, T7, T8) **مبوّبة بقرار بشري**. كتابة الشيفرة الآن ستكون تخميناً. الواجهات ومتطلّبات الأداء ومعايير القبول محدّدة بدقة؛ **والشيفرة تُكتب اختباراً-أولاً عند التنفيذ**.
2. **T3 تُعيد صياغة قواعد السلامة** الواردة في `AGENTS.md` §5.6 — لكن `docs/spec/behaviors/archive-safety.md` هو المرجع التفصيلي، وهذه الخطة تلخّصه. **راجع الملف الأصلي قبل التنفيذ.**
3. **لم أُدرج محتوى `docs/spec/*` نفسه** (23 وثيقة) — الغموض المُبقى منها يبقى. `gap-analysis-notes.md` §12.2 علّم `mrpack_service.py:386` (HTTPS مقابل HTTP) كـ**غير مُتحقَّق** — resolvingه في T7.
4. **المهامTimeouts مرتبطة ببوابة خارجية** (T2). لا يمكن fechas.
5. **`decision-sheet.md` فيه أسئلة مفتوحة** لم تُحسم بعد غير licensing وCurseForge — راجعها قبل T4.