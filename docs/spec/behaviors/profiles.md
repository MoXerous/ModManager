# behaviors/profiles.md — النسخ، والتحقق، والتفعيل/التعطيل، والنسخ الاحتياطي

> **المرجع:** `app\services\profile_manager.py` · `app\domain\profile_aggregate.py` · `app\services\profile_validation.py` · `app\services\profile_content_toggle.py` · `app\services\profile_backup_restore.py` · `app\services\profile_update_backup.py` · `app\services\instance_detector.py`

---

## 1. تصنيف الملكية — `ProfileManager`

### 1-1. أربع طرق إنشاء فقط، كلها تُفوّض لـ`ProfileAggregateRepository.create_profile`

| الطريقة | `profile_kind` | `ownership_policy` | `source_pack` |
|---|---|---|---|
| `create_custom_profile` (46) | `custom` | `app_managed` | — |
| `create_linked_profile` (65) | `linked_instance` إن `instance_mode=="instance"` وإلا `linked_mods_folder` | `user_linked` | — |
| `create_linked_modpack` (89) | `linked_modpack` | `pack_linked` | **مطلوب** |
| `create_legacy_profile` (112) | `legacy` | `legacy_unknown` | — |

### 1-2. تصنيفات مغلقة — `profile_aggregate.py:9-27`

```python
PROFILE_KINDS       = {legacy, custom, linked_instance, linked_mods_folder, linked_modpack}
OWNERSHIP_POLICIES  = {legacy_unknown, app_managed, user_linked, pack_linked}
_KIND_OWNERSHIP     # خريطة 1:1 — كل نوع له سياسة واحدة بالضبط
```

`ProfileAggregateMetadata.__post_init__` (78) يرفض:
- زوج (نوع، سياسة) غير مطابق (101-105)
- `linked_modpack` **بلا** `source_pack` (106-108)
- `source_pack` على أي نوع **غير** `linked_modpack` (109-112)
- `SourcePackIdentity.manifest_sha256` لا يطابق `^[0-9a-f]{64}$` (57-60)

### 1-3. ⚠️ **النتيجة الحرجة: `ownership_policy` لا يُقرأ في أي مسار تعديل**

بحث `ownership_policy` في كل `app\` و`tests\` يظهر في: تعريفات التصنيف، خصائص القراءة في `profile_aggregate.py:267`، وسائط `ProfileManager`، واستمرارية `repositories.py`. **لا بوابة ولا قائمة سماح ولا رفض يقرأه لتفويض أو منع كتابة.**

**التفويض الفعلي مُطبَّق في طبقات أخرى:**

| الطبقة | التفصيل |
|---|---|
| **إعادة فحص نطاق المسار** | `profile_content_toggle._preview_candidate:131` · `profile_content_remove._preview_candidate:177` · `profile_backup_restore:116` |
| **رفض الروابط الرمزية** | toggle 124/88 · remove 170/211 · restore 202/220 · export 490/539/549 · worlds 101/194 |
| **رفض تصادم الوجهة** | toggle 89/138 · remove 225/234 · restore 224 |
| **تصنيف لكل ملف** | `local_mod_classification.py:20-24` — من SHA-1 مقابل `installed_mods.json`، **لا من `ownership_policy`** |
| **معاينة ← تأكيد ← تنفيذ** | بكلمات تأكيد مُنمَّطة: `REMOVE` · `RESTORE` · `IMPORT` |

> **الوثائق المصاحبة تقولها صراحةً:** `ProfileManagerDialog` (4213) و`remove_selected_profile` (13089): «these actions do not delete downloaded files on disk».
> **⚠️ `allow_file_deletion_on_remove`** موجود على `Profile`، يُضبط `False` في `profile_folder_import.py:112`، ويحمله `LegacyProfileSnapshot` — **لم أجد موضع قراءة. غير مُتحقَّق** إن كان مُطبَّقاً.

> **في Rust:** اجعل `ownership_policy` **مُنفِّذاً فعلاً** أو احذفه. حقل «ملكية» لا يُقرأ هوmkdir أنقى أنواع technical debt. **يُحسم قرارك.**

---

## 2. كشف النسخة — `instance_detector.py`

> «Read-only instance folder detection and validation. This service **never creates, deletes, moves, or modifies** user files… Detection is intentionally conservative because launcher layouts differ.»

### 2-1. تحليل الجذر — `_roots_for_selected_folder`

| الشرط | النتيجة |
|---|---|
| الاسم `casefold` ∈ `{".minecraft", "minecraft"}` **و** `parent != selected` **و** `_has_launcher_metadata(parent)` (ملف `mmc-pack.json` \| `instance.cfg` \| `modrinth.index.json`) | **`(parent, selected)`** — جذر النسخة = الأب، جذر ماينكرافت = المجلد المختار. **شكل Prism/MultiMC** |
| غير ذلك | `(selected, _minecraft_root_for(selected))` — أول موجود من `.minecraft` ثم `minecraft` ثم الجذر نفسه |

### 2-2. التخطيطات المُفحوصة — `_default_paths(minecraft_root)`

`mods/` · `shaderpacks/` · `resourcepacks/` · `saves/` · `servers.dat` · `config/` · `screenshots/`
**+ ملفات البيانات الوصفية على جذر النسخة:** `mmc-pack.json` · `modrinth.index.json` (جذر ثم `minecraft_root`) · `launcher_profiles.json` (**`minecraft_root` فقط**)
**+** `minecraft_root/mods/*.jar` (بحد أقصى 30) كبديل.

> `profile_folder_import.py::_detection_root_for_selected_folder` يضيف: إن كان المجلد المختار `mods|config|resourcepacks|shaderpacks|saves`، **يتصعّد مستوى** وإن كان الأب `.minecraft`/`minecraft` وجدّ الأب يحمل بيانات مشغّل ⇒ الجذر = الجدّ، و`selected_folder_role` = اسم المجلد.

### 2-3. شدّة التحقق

| الشرط | الشدّة | السبب |
|---|---|---|
| مسار فارغ | **error** | `"No instance folder was selected."` |
| غير موجود | **error** | `"Instance folder does not exist."` |
| ليس مجلداً | **error** | `"Selected path is not a folder."` |
| جذر ماينكرافت غير موجود | **error** | |
| mods/shaderpacks/resourcepacks/saves مفقودة | **info** | **مقصود**: «Missing optional folders are warnings because some instances create them only after first launch» |
| المسار موجود لكنه ليس مجلداً | **warning** | (عدا `servers.dat`) |
| تعارض إصدار/لودر متوقَّع مقابل مكتشف | **warning** | |
| إصدار أو لودر غير قابل للكشف | **info** | |
| `launcher_profiles.json` غير قابل للقراءة / ليس قاموساً / >1 مرشّح | **warning** / **info** | |
| jar غير-zip أثناء الفحص | **info** | `"Skipped non-zip jar during loader scan: …"` |

### 2-4. `detection_confidence`

| القيمة | الشرط |
|---|---|
| `conflict` | **>1 إصدار مختلف أو >1 لودر مختلف مكتشف** |
| `confirmed` | (`mmc-pack.json` أو `modrinth.index.json` موجود) **و** (إصدار أو لودر موجود) |
| `likely` | إصدار/لودر موجود **بلا** بيانات حزمة |
| `unknown` | غير ذلك |

> ⚠️ **`_choose_value` يعيد أول مفتاح عند التعارض — بلا خطأ، وبلا ترتيب حتمي.**

### 2-5. `validation_status`
`error` إن وُجد خطأ · `warning` إن وُجد تحذير · `ok` إن لم يوجد إلا `info` · `unknown` وإلا.

`validate_manual_paths(...)` يضيف لكل مسار: مفقود ⇒ `info` · ليس مجلداً ⇒ `warning` · **خارج جذر النسخة ⇒ `warning` "{label} is outside the selected instance root."**

> **مُثبَت:** `test_metadata_repair_plan_blocks_conflicting_loader_detection` — `mmc-pack.json` يعلن `net.minecraftforge` **و** `net.neoforged` ⇒ `plan.can_apply is False` مع حاجب يحوي `"conflicting"`.

---

## 3. التحقق من البروفايل — `validate_profile`

> **توقيع:** `validate_profile(profile, *, saved_projects=None, timestamp=None, cancel_callback=None) -> ProfileValidationReport`
> **وصف الدالة (سطر 64):** «Validate profile metadata and local folders **without modifying anything**.»

### 3-1. السلّم

`SEVERITY_OK="OK"` · `SEVERITY_WARNING="Warning"` · `SEVERITY_ERROR="Error"` · `SEVERITY_INFO="Info"`
`overall_status` = **الأسوأ حاضراً** بالترتيب Error → Warning → Info → OK (`_overall_status:474`)

### 3-2. جدول الفحوص الكامل

| الفحص | السطر | الشرط | الشدّة |
|---|---|---|---|
| `Profile root` | 74 / 76 / 78 | غير مضبوط / مفقود أو ليس مجلداً / موجود | **E** / **E** / OK |
| `profile.json` | 148 | مفقود (قاعدة البيانات ما زالت صالحة) | **W** |
| `profile.json` | 151 | موجود وليس ملفاً | **E** |
| `profile.json` | 155 / 157 | يُحلَّل / استثناء | OK / **E** |
| `Minecraft version` | 82 / 84 | مضبوط / مفقود | OK / **E** |
| `Modloader type` | 89 | `profile_loader_issue(...)` غير فارغ (فارغ، أو ∈ `NON_PROFILE_LOADER_VALUES = {any, minecraft, datapack, iris, optifine, canvas}`) | **E** |
| `Modloader type` | 91 / 94 | معروف / غير في `COMMON_PROFILE_LOADERS` | OK / **W** |
| `Detected modloader mismatch` | 102 | `detected_loader != loader` | **W** |
| `mods folder` | 110/160 | مطلوب: غير مضبوط/مفقود ⇒ **E**؛ ليس مجلداً ⇒ **E**؛ غير ذلك OK | |
| `config folder` | 111/160 | **اختياري**: غير مضبوط/مفقود ⇒ **W**؛ ليس مجلداً ⇒ **E** | |
| `installed_mods.json` | 181 | موجود وغير قابل للقراءة | **W** |
| `installed_mods.json` | 184 / 186 | ≥1 سجلّ مدار / 0 | OK / **I** |
| `installed_mods.json` | 189 | سجلّات بلا SHA-1 | **I** |
| `Local mods` | 124 | مجلد mods غير متاح ⇒ التصنيف متخطّى | **I** |
| `Local mods` | 209 | لا ملفات قابلة للمسح | **I** |
| `Local mod file` | 222 | `sha1_file` رفعت استثناء | **W** |
| `Managed mods` | 255 | ≥1 / 0 | OK / **I** |
| `Unknown mods` | 262 / 264 | أي `External / Unknown` / لا شيء | **W** / OK |
| `Duplicate mods` | 266 / 268 | أي / لا شيء | **W** / OK |
| `Incompatible mods` | 270 / 277 | أي / لا شيء | **W** / OK |
| `Disabled mods` | 279 | أي `Disabled` (**يُترك دون مساس**) | **I** |
| `Local mod files` | 281 | عدد غير قابل للقراءة | **W** |
| `File skipped reason` | 421-446 | صفوف لكل ملف، **سقفها `_MAX_FILE_REASON_ROWS = 25`**؛ مجهول/مكرر/غير متوافق ⇒ **W**، معطّل ⇒ **I**؛ التجاوز ⇒ **I** | |
| `Saved project target metadata` | 293 | لا مشاريع محفوظة | **I** |
| `Saved project target metadata` | 324 / 326 | عدم تطابق إصدار ماينكرافت أو لودر / نظيف | **W** / OK |
| `Saved project target metadata` | 328 | حقول ناقصة | **I** |
| `Client/server side` | 337 | لا مشاريع محفوظة | **I** |
| `Client/server side` | 352 / 359 | أي `client_side=="unsupported"` / لا شيء | **W** / OK |
| `Client/server side` | 360 | بيانات جهة غير معروفة | **I** |
| `Missing dependencies` | 371 / 376 | غير قابل للكشف من الميتافيزقا المحفوظة — استخدم مراجعة التبعيات | **I** |
| `Optional dependencies` | 372 / 380 | غير محفوظة كحقيقة صحة للبروفايل | **I** |

### 3-3. قاعدة «بلا أثر جانبي» تصمد

الدالة **تقرأ فقط**. الكتابات الوحيدة في قائمة `items` داخل الذاكرة.
الإلغاء يرفع `RuntimeError("Profile validation cancelled. No files were changed.")` (217).
**لا استيراد شبكة** في هذه الوحدة إطلاقاً.

`test_validate_profile_is_read_only_and_exposed_in_ui` (تأكيد نص مصدر) يؤكد غياب `ModrinthClient` · `write_text` · `shutil.move` · `.unlink(`.

> **⚠️ ترجمة ترجمتنا:** «`config` مفقود ⇒ **تحذير** لكن `mods` مفقود ⇒ **خطأ**» — منطقي لأن `mods` هو الشرط الوجودي الوجودي للنسخة، و`config` اختياري.

---

## 4. التفعيل والتعطيل — `profile_content_toggle.py`

> **نعم — إعادة تسمية إلى `.jar.disabled`. لا نسخ، لا حذف.** `AGENTS.md` §5.12 يؤكّده.

### 4-1. التعطيل — `_toggle_destination` (147)

| # | الحارس | الرسالة |
|---|---|---|
| 1 | المصدر **ملف على المستوى الأعلى** — `if source.parent != root` | `"Only top-level active mod files can be disabled by rename."` |
| 2 | `*.jar.disabled` أصلاً أو داخل `disabled_mods/` | `"File already appears disabled."` |
| 3 | اللاحقة ليست `.jar` | `"Only .jar files can be disabled in this phase."` |
| 4 | — | `Path.rename` إلى `f"{name}.disabled"` (92, 160) |

### 4-2. التفعيل (162)
- `*.jar.disabled` ⇒ اقتطع `.disabled`
- **أو** `.jar` داخل `disabled_mods/` ⇒ **الوجهة `root/<name>`** (يعود للمستوى الأعلى)
- غير ذلك ⇒ `"Only .jar.disabled files or .jar files inside disabled_mods can be enabled in this phase."`

### 4-3. الرفض المُشترك (معاينة 103 + تنفيذ 73)

| الرفض | السطر | الرسالة |
|---|---|---|
| مجلد mods غير قابل للحل | 118 | `Mods folder cannot be resolved: …` |
| مجلد mods مفقود/ليس مجلداً | 121 | `Mods folder does not exist.` |
| المصدر مفقود/ليس ملفاً | 123 | `Source file does not exist.` |
| **رابط رمزي** | 125 (معاينة) / 88 (تنفيذ) | `Symlink source skipped for safety.` |
| **خارج مجلد mods** (`is_relative_to` على مسارات محلولة) | 132 | `Source file is outside the active profile mods folder.` |
| **تصادم الوجهة** | 139 / 90 | `Destination already exists; overwrite is not allowed.` / `Destination already exists; nothing was overwritten.` |

**تحذير غير حاجب** للملفات الخارجية/غير المعروفة (142-143).
`ValueError("Content toggle action must be enable or disable.")` (63).

### 4-4. التراجع عن النجاح الجزئي — `execute_content_toggle_preview` (73-100)

1. أول فشل يضع `failed = True` (96)
2. **كل إدخال مسموح متبقٍ يصبح `"skipped"`** (81-83)
3. `_rollback_completed(completed)` (98) يمشي **بترتيب عكسي** ويعيد التسمية `destination → source`

**الحالات:** `success` · `failed` · `skipped` · `rolled-back` · `rollback-skipped` (تغيّرت الحالة تحته) · `rollback-failed`

**الواجهة:** معاينة تعرض **أول 20** إعادة تسمية + **حتى 12** صفاً محجوباً (10367)، ثم تأكيد Yes/No.

> **هذا يحقق `AGENTS.md` §5.4 (خطة ثم تنفيذ) على مستوى العملية متعددة الملفات** — نموذج يستحق النقل حرفياً.

### 4-5. الحذف المرافق — `profile_content_remove.py`

**سلة-أو-نسخة-احتياطية، لا حذف دائم أبداً:** `"Permanent delete is not supported by this safe remove action."` (135)
النسخ إلى `<mods>/.modrinth_bulk_downloader_removed_content/<timestamp>/files|removed/…` (80, 187-188) مع `remove_manifest.json` (16, 301).
**يتطلب كتابة `REMOVE`** عند اختيار أي ملف خارجي/غير معروف (191).

---

## 5. النسخ الاحتياطي والاستعادة — **نظامان مستقلّان**

### 5-A. نسخ التحديث (لكل ملف، «modrinth-update-downloads»)

عبر `BackupManagerDialog` (2902) ← `manage_update_backups` (7690) ← `BackupScanWorker` ← `BackupRestoreWorker`.

> نصّ التأكيد في `restore_selected_backups` (7728): «Backup files are **copied, not moved**. Existing current files are **never overwritten**. If a filename already exists, a unique suffix is added. **No files are permanently deleted.**»
> ⚠️ **`BackupScanWorker`/`BackupRestoreWorker` في `app/ui/workers.py` — لم يُقرأ. غير مُتحقَّق.**

### 5-B. نسخ البروفايل (كامل، copy-only) — `profile_update_backup.py`

**ما يُنسخ** — `create_profile_update_backup` (48)، `include_saves=False` افتراضياً:

| البند | السطر | ملاحظة |
|---|---|---|
| `profile.json` | 118 | إن كان مفقوداً **تُكتب لقطة ميتافيزقا مولَّدة داخل النسخة فقط** (131-138) |
| `installed_mods.json` | 72 | إن وُجد، وإلا `skipped` |
| `mods/` | 76 | `shutil.copytree(..., symlinks=True)` |
| `config/` | 77 | نفس |
| `saves/` | 82-94 | **فقط مع `include_saves`**، وإلا `skipped` بـ`"Saves are not included by default."` |

> وصف الدالة (55-59): «This is intentionally **copy-only**. It **never** deletes, moves, enables, disables, or overwrites files in the active profile.»

**المكان والتسمية:** `<profile_root>/.modrinth_bulk_downloader_profile_backups/<ختم UTC مُنقّى، ≤64 محرفاً>` (15, 199, 63)
الإيصال: `backup_manifest.json` بمخطط `modrinth_bulk_downloader.profile_update_backup.v1` (16-17)
يتجاهل `APP_TRANSIENT_DIRNAMES` (19-25) وجذر النسخة نفسه (173).

### 5-C. الاستعادة — `profile_backup_restore.py`

| الخطوة | التفصيل |
|---|---|
| `list_profile_update_backups` (74) | يعدّ أدلة الختم، يقرأ `created_at` من `backup_manifest.json`، ترتيب تنازلي (101) |
| **`build_profile_backup_restore_preview` (105)** | **يرفض أي دليل خارج** `<profile_root>/.modrinth_bulk_downloader_profile_backups` (116-117) |
| المدخلات (125-142) | `profile.json` · `installed_mods.json` · `mods/` · `config/` · **`saves/` فقط مع `include_saves`**، وإلا محجوب بـ`"Saves restore is skipped by default."` (140) |
| أسباب الحجب (191-210) | مسار غير مضبوط · مدخل غير موجود · **مدخل رابط رمزي** · ليس ملفاً ولا مجلداً |
| **`execute_profile_backup_restore` (155)** → `_restore_entry` (213) | الوجهة القائمة **تُعاد تسميتها إلى** `<profile_root>/.modrinth_bulk_downloader_restore_safety/<stamp>/current/<label>` (227)، **ثم تُنسخ** النسخة (لا تُنقل). المجلدات المحتوية روابط رمزية مرفوضة (221, 255) |
| عند الفشل | `_rollback_failed_restore` (241) ينقل الهدف الجزئي جانباً **ويستعيد نسخة الأمان** |
| التقدم | 0→100 (178, 181) |
| الإيصال | `restore_manifest.json` داخل مجلد الأمان (275-297) |

**الواجهة:** قائمة ← معاينة ← Yes/No ← **كتابة `RESTORE` طtyped** (7836) ← `ProfileBackupRestoreWorker`.
**الإلغاء افتراضاً «لا» في كل مكان.**

> **مبدأ ممتاز يُنقل:** `copy` لا `move` + وجهة قائمة تُحفظ جانباً أولاً + نسخة أمان ثانوية. **ثلاث طبقات حماية قبل أي استعادة.**

---

## 6. إحصاءات المخزون

`ContentInventorySnapshot` يشتقّ: `managed_count` · `external_count` · `disabled_count` · `duplicate_count` · `incompatible_count` · `unknown_compatibility_count` · `missing_managed_count`.

| البند | القيمة |
|---|---|
| `OWNERSHIP_MANAGED` / `OWNERSHIP_EXTERNAL` | `managed` / `external` |
| `ACTIVATION_ENABLED` / `ACTIVATION_DISABLED` | `enabled` / `disabled` |
| `COMPATIBILITY_*` | `compatible` / `incompatible` / `unknown` |
| `LocalContentFact` | `ordinal, path, filename, size, sha1="", resource_type="unknown", provider="", project_id="", version_id="", version_number="", game_versions=(), loaders=(), disabled=False` |
| `ManagedContentFact` | `record_key, provider, project_id, version_id, sha1="", filename=""` |
| `ContentInventoryEntry` | `local, ownership, ownership_evidence, activation, duplicate_keys=(), compatibility, compatibility_reason, managed_record_key="", hash_mismatch=False` |
| `ContentInventoryIssue` | `kind, message, local_ordinal=None, managed_record_key=""` |

---

## 7. اقتراح للنقل إلى Rust (غير مُلزِم)

| البند | القرار |
|---|---|
| `ownership_policy` | **نفّذه أو احذفه.** اقترح `enum Ownership { AppManaged, UserLinked, PackLinked, Unknown }` مع دالة `permits(delete: bool)` |
| 4 فحوص الـloader/لودر/إصدار/مجلدات | §`validation.rs` مع `enum Severity` |
| سقف 25 صف «سبب تخطّي» | **احتفظ** — يمنع إغراق الواجهة |
| التفعيل/التعطيل | `fs::rename` + نفس الحرّاس السبعة + **تراجع عكسي** |
| `Profile` قابل للتعديل (22 حقلاً) + `ProfileAggregate` مُجمَّد | **استعمل `struct` واحدة，غير قابلة للتعديل**. راجع `data-model.md` §5 |
| 5 أعمدة «أين الـmods» | **وحّدها في `profile_root` واحد** |
| نسخ البروفايل | `copytree` + وجهة الأمان + إيصال — **نفس الطبقات الثلاث** |

## 8. غير مُتحقَّق

| البند |
|---|---|
| لم أُشغّل أي اختبار |
| `app/ui/workers.py` لم يُقرأ — `BackupScanWorker`/`BackupRestoreWorker` |
| `allow_file_deletion_on_remove` — **لم أجد موضع قراءة** |
| `profile_materialization.py` · `profile_metadata_repair.py` · `profile_runtime_artifacts.py` · `profile_workspace_summary.py` لم تُقرأ |
| سلوك الإلغاء الدقيق لـ`execute_profile_backup_restore` |
