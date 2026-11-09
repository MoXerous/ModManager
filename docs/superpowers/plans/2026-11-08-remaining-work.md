# خطة الخطوات الباقية — ModManager

- **التاريخ:** 2026-11-08
- **الفرع:** `feat/remaining-work` (من `feat/task5a-ui-foundation` @ `90c512a`)
- **المنفّذ:** وكيل واحد، بلا رجوع للمستخدم إلا عند حاجز خارجي حقيقي.
- **المرجع الأعلى:** `AGENTS.md`، ثم `docs/foundations.md`، ثم `docs/spec/`، ثم `docs/adr/`.

> **نتيجة الفحص قبل الكتابة:** `docs/todo.md` **متقادم في مواضع كثيرة**: بنودٌ تقول «لا كود بعد» وكودُها موجود
> (`pending_pack_recoveries`، `recover_pack_apply`، `PackRecoveryBanner`، مفاتيح `recovery.pack.*`،
> `ProviderError::Unsupported`، معالجة `resolve` للخطأ، `write_atomic_unique` لـ`profile.json`).
> ولذلك **المهمة الأولى تدقيقُ الوثائق لا تنفيذُ بنودها**. أمّا الثغرات الحقيقية التي أثبتها الفحص فهي:
>
> 1. **لا كاتب إنتاجيّ لـ`instance.lock.json`** — `write_lock` موجود ويُستدعى في الاختبارات فقط،
>    و`write_lock` نفسه يكتب بـ`std::fs::write` لا كتابةً ذرّية.
> 2. **لا كاتب إنتاجيّ لجدول `installed_mods`** — الجدول والهجرة M4 موجودان، ولا صفَّ يُكتب بعد التثبيت.
> 3. **لا فحص تحديثات** — `F-120` وما بعدها «Keep / M2-M3» ولا كود.
> 4. **الشريحة 0 للترحيل (F-181) غير موجودة** — وهي «الشريحة التالية» بنصّ `todo.md`.
> 5. **بطاقة المسارات/التشخيص في الإعدادات** غير موجودة (تعليق `SettingsPage.tsx` يعد بها في 5c).
>
> أمّا «نقل قوائم الإصدار/اللودر إلى core» فالقرار: **لا يُنقل**. القائمة اقتراحات مفتوحة لا مجموعة مغلقة،
> ومصدرها الصحيح وسوم Modrinth، ولا يوجد fixture موثّق لشكلها؛ نقلُ نسخة ثانية إلى core يضاعف الصيانة بلا فائدة.
> يُحدَّث البند في `todo.md` بقرار صريح.

---

## قواعد التنفيذ في كل مهمة

1. **TDD:** يُكتب الاختبار الفاشل أولًا، ويُشغَّل ويُرى أحمر، ثم يُكتب الكود حتى الأخضر.
2. **commit واحد لكل مهمة** بصيغة Conventional Commits، والرسالة إنجليزية.
3. **البوابة بعد كل مهمة تلمس Rust:**
   ```
   cargo fmt --all -- --check
   cargo clippy --workspace --all-targets -j 1 -- -D warnings
   cargo test --workspace -j 1
   ```
   وبعد كل مهمة تلمس الواجهة: `pnpm exec tsc --noEmit && pnpm build`.
4. **بيئة الجهاز:**
   ```
   $env:CARGO_TARGET_DIR = 'F:\agent-temp\opencode\mm-target'
   $env:CARGO_PROFILE_DEV_DEBUG = '0'
   $env:CARGO_INCREMENTAL = '0'
   ```
   ولا أمرا cargo معًا. ولا `pnpm dev` أثناء بناء cargo.
5. **لا تبعية جديدة.** `rusqlite` يحمل feature `backup` أصلًا.
6. **لا `unwrap()`/`expect()` في كود الإنتاج.** الاختبارات وحدها تستثنيهما.
7. **كل نص واجهة جديد** يُضاف إلى `src/i18n/ar.json` و`en.json` معًا، ويُغطّى باختبار مفتاح.
8. **بعد أي تغيير واجهة:** اختبار تصميم في المتصفّح المدمج بصفحة معاينة مؤقتة تعرّف
   `window.__TAURI_INTERNALS__`، ثم تُحذف قبل الـcommit.

---

## المهمة 0 — تدقيق الوثائق وإغلاق البنود المتقادمة

**الملفات:** `docs/todo.md`، `docs/adr/0005-pack-url-policy.md` (جديد)، `docs/foundations.md` (إن لزم).

**ما يُفعل:**

1. تصحيح جدول الحالة: M3 «قيد العمل» بسبب **التحديثات وكاتب القفل** لا بسبب الحزم.
2. حذف/تصحيح البنود المتقادمة مع ذكر الـcommit الذي أغلقها:
   - `pending_pack_recoveries` / `recover_pack_apply` / `dismiss_pack_apply` / `PackRecoveryBanner` / مفاتيح `recovery.pack.*` → موجودة (`682a1a9` وما قبله).
   - `ProviderError::Unsupported` → موجود (`core/src/providers/mod.rs:106`).
   - مُبتلِع الخطأ في `resolve` → أُصلح؛ الخطأ صار blocker لا خريطة فارغة.
   - `.part` في `profile.json` → أُصلح بـ`write_atomic_unique` (`f7cb5fb`).
   - رابط الحزمة كاحتياط: **الكود يحسمه أصلًا** (`exec.rs::candidate_urls`: رابط المزوّد أولًا ثم روابط الحزمة).
3. كتابة `ADR 0005` لقرار رابط الحزمة: المزوّد أولًا، ثم روابط الحزمة المتطابقة الهاش، مع بقاء كل تنزيل محكومًا بالهاش.
4. تسجيل «نقل قوائم الإصدار/اللودر» كقرار **مرفوض** مع سببه أعلاه.
5. إضافة قسم «ما تبقّى فعلًا» يطابق مهام هذه الخطة.

**الاختبار:** لا كود. البوابة: `git diff --check` + مراجعة بشرية للنص.

**commit:** `docs: align the remaining-work list with the shipped code`

---

## المهمة 1 — قارئ الترحيل: الشريحة 0 (F-181)

**الهدف:** يجد جذر التطبيق القديم، يفتح **نسخة** من `app.sqlite3` بالـbackup API من اتصال **قراءة فقط**،
يقرأ `profiles` و`profile.json` و`installed_mods.json`، ويخرج بتقرير **لا يكتب في الجذر القديم ولا في
قاعدة التطبيق الجديدة**. لا قراءة تلقائية: لا يُستدعى إلا بأمر صريح من الواجهة.

**الملفات الجديدة:**

```
core/src/legacy/mod.rs        // الواجهة العامة + الأنواع
core/src/legacy/discover.rs   // حلّ الجذور
core/src/legacy/db.rs         // snapshot + PRAGMA + قراءة profiles
core/src/legacy/json.rs       // profile.json + installed_mods.json
core/src/legacy/tests.rs      // F-181
```

وتُضاف `pub mod legacy;` إلى `core/src/lib.rs`.

**الأنواع الأساسية:**

```rust
pub enum LegacyRootSource { Explicit, Environment, LegacyDotDir, Platform, None }

pub struct LegacyRoots {
    pub source: LegacyRootSource,
    pub data: PathBuf,
    pub config: PathBuf,
    pub cache: PathBuf,
    pub logs: PathBuf,
    pub db_file: PathBuf,
}

pub struct LegacyScan {
    pub roots: LegacyRoots,
    pub database: Option<LegacyDbScan>,
    pub profiles: Vec<LegacyProfileScan>,
    pub warnings: Vec<LegacyWarning>,
}

pub struct LegacyDbScan {
    pub path: PathBuf,
    pub bytes: u64,
    pub user_version: u32,
    pub supported: bool,          // user_version <= 2
    pub has_profiles_table: bool,
    pub profile_count: usize,
    pub columns: Vec<String>,     // PRAGMA table_info(profiles)
}

pub struct LegacyProfileScan {
    pub id: String,
    pub name: String,
    pub game_version: String,
    pub loader: String,
    pub kind: String,             // profile_kind أو ""
    pub ownership: String,        // ownership_policy أو ""
    pub root: PathBuf,
    pub root_exists: bool,
    pub profile_json: JsonFact,
    pub installed_mods: JsonFact,
}

pub struct JsonFact {
    pub present: bool,
    pub schema: String,
    pub ok: bool,
    pub detail: String,           // سبب الفشل، بلا مسار حساس
}
```

**حلّ الجذور (`discover`)** بهذا الترتيب، وينتهي عند أول مرشّح:

1. `explicit: Option<&Path>` — من الأمر (اختيار المستخدم أو الاختبار).
2. `MODRINTH_BULK_DOWNLOADER_HOME` — `<root>/{data,config,cache,logs}`.
3. `~/.modrinth_bulk_downloader` — **فقط** إذا وُجد `data/app.sqlite3` (توافق القديم).
4. مسارات المنصّة:
   - Windows: `%LOCALAPPDATA%\ModrinthBulkDownloader` للبيانات/الكاش/السجلات، `%APPDATA%\ModrinthBulkDownloader` للإعداد.
   - Linux: `$XDG_DATA_HOME` أو `~/.local/share` للبيانات، `$XDG_CONFIG_HOME` أو `~/.config` للإعداد، `$XDG_CACHE_HOME` للكاش، `$XDG_STATE_HOME` للسجلات.
   - macOS: `~/Library/Application Support/ModrinthBulkDownloader`، `~/Library/Caches/...`، `~/Library/Logs/...`.

> لا يدعم القارئ `portable.flag` لأن جذر التطبيق القديم غير معروف من تطبيقنا؛ يُذكر ذلك في `warnings` عند غياب القاعدة.

**الـsnapshot (`db::snapshot`):**

```rust
pub fn scan(roots: &LegacyRoots, scratch: &Path) -> Result<LegacyScan, LegacyError> {
    if !roots.db_file.is_file() { /* scan بلا database + warning */ }
    let src = Connection::open_with_flags(&roots.db_file, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let dest = scratch.join(format!("legacy-{}.sqlite3", unique_suffix()));
    let mut dst = Connection::open(&dest)?;
    Backup::new(&src, &mut dst)?.run_to_completion(5, Duration::from_millis(50), None)?;
    drop(dst); drop(src);
    let snap = Connection::open_with_flags(&dest, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    // PRAGMA user_version، PRAGMA table_info(profiles)، SELECT المحكوم بالأعمدة الموجودة
    // ...
    // يحذف dest قبل return، ويحذفه أيضًا عند الخطأ (guard)
}
```

- `PRAGMA table_info(profiles)` يحدّد الأعمدة الموجودة؛ **لا DDL مفترض** (v1 = 23 عمودًا، v2 = 34).
- `user_version > 2` ⇒ `supported = false`، ولا تُقرأ الصفوف؛ تحذير `UnsupportedDbVersion`.
- إن غاب جدول `profiles` ⇒ `has_profiles_table = false` وتحذير، لا فشل.
- جذر البروفايل يُحسب بنفس سلّم القديم:
  `profile_root` → `instance_root` → `minecraft_root` → أب `mods_dir` إن كان اسمه `mods` → أب `download_dir` إن كان اسمه `mods` → `download_dir`.

**قراءة JSON (`json`):**

- سقف **4 MiB** لكل ملف (كما في القديم)، ورفض NUL في المسارات.
- `profile.json`: يقبل مخططين:
  - `modrinth_bulk_downloader.profile_metadata.v1` (كائن `profile` بداخله `paths.profile_root`).
  - `modrinth_bulk_downloader.profile.v2` (قديم؛ الحقول في الجذر أو داخل `profile`).
  - أي مخطط آخر ⇒ `ok=false`, `schema` الخام, سبب `unsupported_schema`.
- `installed_mods.json`: يتطلب `modrinth_bulk_downloader.installed_mods.v1`، ويقرأ `managed_mods`،
  **ويتخطّى** `managed_status ∈ {external, "external / unknown", unknown}` وكل سجل بلا `project_id` أو `version_id`،
  ويعدّ المُتخطّى في `detail` بصيغة `managed=<n>;skipped=<m>`.
- أي خطأ JSON ⇒ `ok=false` وتحذير، **ولا يُسقط** بقية البروفايلات.

**الاختبارات (أسماء تحمل F-181):**

1. `spec_f181_missing_root_reports_none_and_writes_nothing`
2. `spec_f181_snapshot_is_a_copy_and_legacy_root_is_untouched` — شجرة ملفات قبل/بعد + عدم وجود ملفات جديدة في الجذر.
3. `spec_f181_reads_v2_profiles_and_columns`
4. `spec_f181_reads_v1_schema_without_assuming_columns`
5. `spec_f181_newer_database_is_reported_unsupported_without_rows`
6. `spec_f181_reads_profile_json_and_installed_mods`
7. `spec_f181_skips_external_and_unknown_manifest_records`
8. `spec_f181_corrupt_json_is_a_warning_not_a_failure`
9. `spec_f181_snapshot_is_removed_after_scan`

**commit:** `feat(legacy): scan the old app database without writing to it`

---

## المهمة 2 — أمر IPC وبطاقة الإعدادات للترحيل

**الملفات:** `src-tauri/src/commands/legacy.rs` (جديد)، `src-tauri/src/commands.rs` (تسجيل `pub mod legacy;`)،
`src-tauri/src/lib.rs` (handler)، `src-tauri/src/dto.rs`، `src/lib/ipc.ts`،
`src/app/pages/SettingsPage.tsx`، `src/i18n/{ar,en}.json`.

**العقد:**

```rust
#[tauri::command]
pub async fn scan_legacy_import(override_home: Option<String>) -> Result<LegacyScanDto, IpcError>
```

- يعمل داخل `spawn_blocking`، ويُنشئ مجلّد snapshot تحت `std::env::temp_dir()` ثم يحذفه.
- `override_home` مسار مطلق فقط؛ غير ذلك `InvalidInput`.
- **لا يُستدعى تلقائيًا** عند الإقلاع (F-181: بلا قراءة تلقائية).
- `LegacyScanDto` يطابق core 1:1، ومسارات جذر البروفايل تُعاد للواجهة لأنها بيانات المستخدم على جهازه.

**الواجهة:**

- بطاقة جديدة في `SettingsPage` بعنوان `settings.legacy.title`.
- زر `settings.legacy.scan` → النتيجة:
  - مصدر الاكتشاف، مسار القاعدة، `user_version`، عدد البروفايلات.
  - جدول بروفايلات: الاسم، إصدار ماينكرافت، اللودر، حالة `profile.json` و`installed_mods.json`.
  - تحذيرات مترجمة من مفاتيح `settings.legacy.warning.*`.
  - سطر صريح: «معاينة فقط — لم يُستورد ولم يُكتب أي ملف» (`settings.legacy.readOnly`).
- حالات: `null` (لم يُفحص بعد)، `loading`، خطأ `IpcError`، نتيجة فارغة.
- RTL: كل الصفوف بخصائص منطقية (`ms-`, `text-start`)، والمسارات `dir="ltr"` و`break-all`.

**الاختبارات:**

- Rust: أمر يفكّ `override_home` نسبيًا برفض، ويمرّر مسارًا مطلقًا إلى core (seam `scan_with`).
- Rust: مفاتيح i18n المستخدمة موجودة في اللغتين (اختبار المفاتيح القائم).
- TS: `tsc` + `pnpm build`.
- متصفّح: البطاقة في العربية والإنجليزية، بلا تجاوز أفقي، والجدول يقبل مسارًا طويلًا.

**commit:** `feat(settings): preview the old app data through a read-only scan`

---

## المهمة 3 — كتابة `instance.lock.json` ذرّيًا وتسجيل التثبيتات

**المشكلة المثبتة:** `instances::write_lock` يكتب بـ`std::fs::write`، ولا يستدعيه كود إنتاج،
وجدول `installed_mods` بلا كاتب. فالتثبيت لا يترك أثرًا يمكن فحص التحديثات عليه.

**الملفات:** `core/src/instances/lock.rs`، `core/src/instances/record.rs` (جديد)، `core/src/instances/mod.rs`،
`core/src/db/mod.rs` (إن لزم seam)، `src-tauri/src/commands.rs`، `src-tauri/src/commands/pack.rs`.

**3-أ — الكتابة الذرّية للقفل:**

- استبدال `std::fs::write` بـ`crate::safety::atomic::write_atomic_unique`.
- `LockError` يضيف `Atomic(#[from] AtomicError)`.
- اختبار: كتابة فوق قفل موجود تنجح، ولا تترك temp، وملف `.part` بائد لا يمنعها.

**3-ب — `instances::record`:**

```rust
pub struct InstalledRecord {
    pub provider: String,
    pub project_id: String,
    pub version_id: String,
    pub file_name: String,
    pub sha1: String,
    pub sha512: String,
}

pub fn record_install(db: &Db, instance_root: &Path, records: &[InstalledRecord]) -> Result<usize, RecordError>
```

- يجد صفّ النسخة بـ`db::instances::find_by_path`؛ غير مسجّلة ⇒ `RecordError::Unregistered`.
- **القفل:** يقرأ الموجود (يتجاهل `NewerSchema`؟ لا: يفشل مغلقًا)، ويستبدل السجل بنفس `filename`
  (مقارنة ASCII case-insensitive)، ثم يكتب مرتّبًا بالاسم عبر `write_lock`. لا حذف لسجلات أخرى.
- **القاعدة:** داخل معاملة `BEGIN IMMEDIATE`، لكل سجل `project_id` غير فارغ:
  `DELETE FROM installed_mods WHERE instance_id=?1 AND provider=?2 AND project_id=?3` ثم `INSERT`.
  السجلات بلا `project_id` تُكتب في القفل ولا تُدخل في الجدول (هوية غير معروفة لا تُخترع).
- `source` في الجدول = `'managed'`.
- اختبارات:
  1. `spec_f128_record_install_writes_lock_and_db`
  2. `spec_f128_record_install_is_idempotent`
  3. `spec_f128_record_install_keeps_unknown_project_in_lock_only`
  4. `spec_f128_record_install_refuses_unregistered_instance`
  5. `spec_f128_write_lock_is_atomic_and_survives_stale_temp`

**3-ج — الاستدعاء بعد التثبيت المباشر:**

- في `execute_install_with` بعد نجاح `execute_batch`، تُبنى `InstalledRecord` من `plan.entries` + `plan.versions`
  (المزوّد من `provider.provider_id()`، والهاشان من `ProjectFile`)، ثم `state.with_db`.
- فشل التسجيل **لا يُحوّل تثبيتًا ناجحًا إلى فاشل**؛ يُسجَّل `tracing::warn!` ويعود `skipped`؟ لا:
  القفل سجل أمانة؛ إن فشل نُبلّغ الواجهة بتحذير في `InstallResultDto` (`recorded: bool`). القرار:
  الخطأ **يُعاد** لأن تثبيتًا بلا سجل يجعل التحديث الأعمى ممكنًا، وهو ما تمنعه §5.5. يُضاف `warning` في
  نتيجة النجاح بدل إسقاط الملفات؟ الأبسط والأصدق: `record_install` قبل إغلاق العملية، وفشله `IpcError`؛
  الملفات نزلت بالفعل، والرسالة تقول «نُزّلت لكن لم تُسجَّل».
  > **قرار الخطة:** يُعاد الخطأ مع رمز `internal` ونصّ واضح. لا كتم.

**3-د — الاستدعاء بعد تطبيق الحزمة:**

- في `commands/pack.rs` بعد `Ok(report)` وقبل إرجاع DTO:
  لكل `report.written` بمصدر `Download`، ابحث عن `InstallOp::Download` بنفس `safe_path` وخُذ `sha512`،
  والاسم من آخر مقطع في `safe_path`، و`provider = "modrinth"` (صيغة `.mrpack` لا تُنتج غيرها).
  مشروع/إصدار فارغان لأن الفهرس لا يحملهما؛ القفل يحمل الاسم والهاش.
- نفس المهمة في مسار الاسترجاع (`recover`)? **لا** — الاسترجاع يكمل تطبيقًا سُجّل مرة؛ تكرار التسجيل idempotent
  لكنه يحتاج `plan` من الأرشيف. يُترك للشريحة التالية مع تعليق `shortcut:` صريح.

**commit:** `feat(instances): record installed files in an atomic lock and the database`

---

## المهمة 4 — فحص التحديثات (معاينة فقط، F-120/F-128/F-129)

**الهدف:** زر «فحص التحديثات» في تفاصيل النسخة يقرأ `instance.lock.json`، يسأل المزوّد عن أحدث إصدار
لكل هاش (SHA-512 أولًا ثم SHA-1)، ويعرض «متاح/محدّث/غير معروف». **لا يكتب شيئًا.**

**الملفات:** `core/src/updates/mod.rs` (جديد)، `core/src/lib.rs`، `src-tauri/src/commands.rs`،
`src-tauri/src/dto.rs`، `src/lib/ipc.ts`، `src/app/pages/InstanceDetails.tsx`، `src/i18n/{ar,en}.json`.

**الأنواع:**

```rust
pub enum UpdateStatus { Available, UpToDate, Unidentified, Unsupported }

pub struct UpdateCandidate {
    pub filename: String,
    pub provider: String,
    pub project_id: String,
    pub current_version_id: String,
    pub current_sha1: String,
    pub current_sha512: String,
    pub latest_version_id: Option<String>,
    pub latest_version_number: Option<String>,
    pub latest_filename: Option<String>,
    pub latest_sha1: Option<String>,
    pub latest_sha512: Option<String>,
    pub status: UpdateStatus,
}

pub struct UpdatePreview {
    pub checked: usize,
    pub candidates: Vec<UpdateCandidate>,
    pub warnings: Vec<UpdateWarning>, // Unsupported, EmptyLock, LockUnreadable
}
```

**الخوارزمية:**

1. اقرأ القفل؛ غيابه ⇒ `warnings = [EmptyLock]` و`checked = 0`.
2. رشّح السجلات التي `file_name` ينتهي بـ`.jar` أو `.jar.disabled` ولها `sha512` أو `sha1`.
3. **SHA-512 أولًا**: استدعِ `get_versions_by_hashes(hashes, Sha512, Some(LatestFilter { loaders: [loader], game_versions: [game_version] }))`
   مرة واحدة. المزوّد الذي يعيد `Unsupported` ⇒ كل السجلات `Unsupported` + تحذير؛ **لا** يهبط إلى «لا تحديث».
4. ما لم يُعده المزوّد: جرّب SHA-1 إن وُجد. ما بقي ⇒ `Unidentified`.
5. المقارنة: إن كان ملف الإصدار الأحدث يطابق هاش السجل ⇒ `UpToDate`، وإلا `Available`.
6. `checked` = عدد السجلات المؤهلة.

**الأمر:** `check_instance_updates(instance_root: String, game_version: String, loader: String)`:
- يرفض لودر `any` أو فارغًا وإصدارًا فارغًا (كما القديم §2-1).
- يبني المزوّد من `req.provider` (نفس محدد `providers::build`) — يُضاف الحقل إلى الطلب.
- لا يكتب شيئًا؛ لا Journal.

**الواجهة في `InstanceDetails`:**

- زر `details.updates.check` بجانب أزرار التفعيل.
- بعد النتيجة: شريط ملخّص `details.updates.summary` (عدد المتاح/المحدّث/المجهول)، جدول
  الاسم الحالي → الإصدار الأحدث، وشارة حالة.
- سطر `details.updates.readOnly` صريح.
- فشل `Unsupported` يظهر كتحذير مترجم لا كخطأ عام.
- لا زر تثبيت بعد: **التثبيت الآمن للتحديثات يبقى خارج هذه الشريحة** لأنه يحتاج نسخًا احتياطية وثلاثية
  Yes/No/Cancel وإعادة تحقق؛ يُسجّل كـ`shortcut:` في الكود وبندًا في `todo.md`.

**الاختبارات:**

1. `spec_f120_check_updates_maps_available_and_current`
2. `spec_f120_check_updates_reports_unidentified_not_current`
3. `spec_f120_check_updates_reports_unsupported_provider`
4. `spec_f120_check_updates_requires_concrete_loader_and_version`
5. `spec_f120_check_updates_writes_nothing` — شجرة القفل قبل/بعد.
6. `spec_f120_check_updates_uses_sha512_first`

**commit:** `feat(updates): preview available updates from the instance lock`

---

## المهمة 5 — بطاقة المسارات والتشخيص في الإعدادات

**الهدف:** تعويض وعد تعليق `SettingsPage` («Paths / Diagnostics») بلا اختراع شبكة/أعلام.

**الملفات:** `core/src/paths.rs` (لا تغيير غالبًا)، `src-tauri/src/commands.rs`، `src-tauri/src/dto.rs`،
`src/lib/ipc.ts`، `src/app/pages/SettingsPage.tsx`، `src/i18n/{ar,en}.json`.

**العقد:**

```rust
pub struct AppPathsDto {
    pub data: String,
    pub config: String,
    pub cache: String,
    pub logs: String,
    pub db_file: String,
    pub db_bytes: u64,
    pub db_version: u32,
    pub app_version: String,
}

#[tauri::command] pub fn app_paths(state: State<AppState>) -> Result<AppPathsDto, IpcError>
#[tauri::command] pub fn open_app_folder(state: State<AppState>, kind: String) -> Result<(), IpcError>
```

- `open_app_folder` يقبل `data|config|cache|logs` فقط، ويحلّ المسار من `Paths` في Rust، **لا من الواجهة**.
- `db_version` من `PRAGMA user_version`، و`db_bytes` من `metadata` (0 إن فشل).
- بطاقة `settings.paths.title` بصفوف: البيانات، الإعدادات، الكاش، السجلات، القاعدة؛ كل صف بزر «فتح»
  للمجلد (والقاعدة تفتح مجلدها). سطر `settings.paths.note` يشرح أنَّ القاعدة محلية لا تُزامَن.
- بطاقة `settings.diagnostics.title`: إصدار التطبيق، إصدار المخطط، حجم القاعدة.

**الاختبارات:**

- `spec_f151_app_paths_reports_the_resolved_roots`
- `spec_f151_open_app_folder_rejects_unknown_kind`
- مفاتيح i18n في اللغتين.
- اختبار متصفّح: البطاقتان RTL/LTR، أزرار الفتح معطّلة/مخفية في المعاينة (استدعاء mock).

**commit:** `feat(settings): show app paths and diagnostics with safe open actions`

---

## المهمة 6 — البوابة النهائية والتوثيق والدفع

1. تشغيل البوابة كاملة (fmt, clippy, test, tsc, build) وتسجيل الأرقام.
2. اختبار التصميم في المتصفّح المدمج: Settings (العربية/الإنجليزية) + InstanceDetails (جدول التحديثات).
3. تحديث `docs/todo.md` بالحالة الحقيقية: ما أُنجز، وما بقي، وما هو **حاجز خارجي**.
4. `git push -u origin feat/remaining-work` ومتابعة CI حتى الأخضر.
5. تقرير عربي وفق `AGENTS.md` §8.

---

## خارج النطاق — بحاجز خارجي حقيقي (يُوثَّق لا يُدّعى)

| البند | الحاجز |
|---|---|
| تثبيت التحديثات آليًا (F-121..F-124) | يحتاج تصميم نسخة احتياطية/ثلاثية تأكيد؛ أُجّل بوعي بعد المعاينة. |
| CurseForge الحيّ (بحث/تنزيل) | يحتاج `CURSEFORGE_API_KEY` بشريًا؛ الاختبارات تبقى `wiremock`. |
| ضغط زر حقيقي داخل نافذة Tauri | يحتاج نافذة مرئية/إنسانًا؛ الطبقة المغطّاة هي أوامر IPC عبر `Channel`. |
| التحديث التلقائي وتوقيع الكود والمثبّتات (M4 الإصدار) | يحتاج مفاتيح توقيع وحسابات توزيع وشهادات. |
| `portable.flag` في القارئ | جذر التطبيق القديم غير معروف من تطبيقنا؛ يُدعم التجاوز الصريح. |
| قوائم الإصدار/اللودر من وسوم Modrinth | لا fixture موثّق لشكل الوسوم؛ القائمة الحالية اقتراحات مفتوحة. |
