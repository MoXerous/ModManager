# behaviors/import-export.md — تصدير/استيراد البروفايل

> **المرجع:** `app\services\profile_export.py` (572 سطراً) · `app\services\profile_folder_import.py` · `app\services\profile_metadata.py` · `app\services\profile_mod_export.py` · `app\services\profile_worlds.py`

---

## 1. صيغة التصدير

| الثابت | القيمة |
|---|---|
| `PROFILE_EXPORT_SCHEMA` | `"modrinth_bulk_downloader.profile_export.v1"` |
| `PROFILE_EXPORT_MODE_METADATA` | `"metadata"` — الوسم **«Metadata only»** |
| `PROFILE_EXPORT_MODE_MODS_CONFIG` | `"mods_config"` — الوسم **«Metadata + mods + config»** |

---

## 2. التصدير — `build_profile_export_plan` (240) + `write_profile_export_zip` (329)

### 2-1. المحتوى المُعاد

| # | البند | السلوك |
|---|---|---|
| 1 | `profile.json` | **مُولَّد** من الميتافيزقا الحالية — **لا يُقرأ من مجلد البروفايل إطلاقاً** (261-268) |
| 2 | `installed_mods.json` | **منسوخ** إن وُجد (273-280) |
| 3 | `mods/**` · `config/**` | **فقط** في وضع `mods_config` (284-300) |
| 4 | `export_manifest.json` | ملخّص مُولَّد (302-314، `_export_manifest_content:613`) |

### 2-2. آلية الكتابة — `write_profile_export_zip` (329)

1. ملف مؤقت `.{name}.zip.tmp` بـ`unique_path` (346)
2. **يُعاد فتح الأرشيف وفحصه** قبل النشر (361-362)
3. `temp.replace(output_path)` (363)
4. عند أي استثناء ⇒ حذف المؤقت (364-367)

> وصف الدالة (335-339): «**The source profile is never modified.**»
> `build_profile_export_plan` **قراءة فقط** (246).

### 2-3. **ما لا يُصدَّر أبداً** — القائمة الحاسمة

**مجلدات** (`PROFILE_EXPORT_EXCLUDED_DIR_NAMES`, 34-47):
`.git` · `.hg` · `.svn` · `__pycache__` · `logs` · `crash-reports` · `crash_reports` · `.modrinth_bulk_downloader_backup` · `.modrinth_bulk_downloader_removed_content` · `.modrinth_bulk_downloader_restore_safety` · `.modrinth_bulk_downloader_staging`

**ملفات** (`PROFILE_EXPORT_EXCLUDED_FILE_NAMES`, 49-56):
`.env` · `.env.local` · `.env.development` · `.env.production` · `.env.test` · `.modrinth_bulk_downloader_mrpack_install.json`

**لاحقات** (`PROFILE_EXPORT_EXCLUDED_SUFFIXES`, 58-72):
`.cer .crt .der .env .gpg .jks .key .keystore .log .p12 .pem .pfx .pgp`

**رموز أسماء حسّاسة** (`PROFILE_EXPORT_SENSITIVE_NAME_TOKENS`, 74-87):
`account, activation, auth, credential, credentials, license, password, passwd, private, secret, session, token`
> تُطابَق كـ**tokens أبجدية-رقمية من اسم الملف** (588-590) — أي `my_token_backup.txt` يُرفض، و`tokenizer.jar` **لا** (ليس tokenاً منفصلاً).

**وأيضاً:**
- **`saves/`** — `include_saves=False` دائماً (253-258)، السبب: `"Saves are excluded by default and are not supported by this export phase."`
- **الروابط الرمزية** (491, 539, 549) — `"Symlink skipped to avoid leaking files outside the profile."`
- غير الملفات/المجلدات/المفقود (484-492, 520-529) — يُسجَّل مستبعَداً بسبب

**هل هناك ما يجب استبعاده ولم يُستبعد؟** لا شيء لا شيء — `PROFILE_EXPORT_JSON_MAX_BYTES = 4 MiB` سقف لكل JSON.

### 2-4. حمولة `export_manifest.json` (620-642)

```json
{ "saves": "excluded by default", "logs": "excluded",
  "crash_reports": "excluded", "secrets_env_keys_private_files": "excluded" }
```
> **جعلت السياسة **قابلة للقراءة آلياً** داخل الأرشيف. نمط يستحق النقل.**

---

## 3. الاستيراد —_metadata فقط

### 3-1. `preview_profile_export_zip_import` (154)

> وصف الدالة (155): «Read this app's profile export zip **without extracting or writing files**.»

| # | الشرط | النتيجة |
|---|---|---|
| 1 | موجود وملف وينتهي `.zip` | وإلا رفض |
| 2 | `inspect_zip_archive` ⇒ جرد بأسماء آمنة | `SafeArchiveError` |
| 3 | **يجب** يحتوي `export_manifest.json` | `"not recognized as this app's profile export"` |
| 4 | **يجب** يحتوي `profile.json` | — |
| 5 | مخطط الـmanifest = `PROFILE_EXPORT_SCHEMA` | وإلا رفض |
| 6 | `include_saves` صحيح ⇒ **تحذير** بأن الـsaves تُتجاهل | 175 |
| 7 | `installed_mods.json` اختياري؛ إن وُجد مخططه يجب = `INSTALLED_MODS_SCHEMA` (399)؛ والسجلّات ذات `managed_status ∈ {external, external / unknown, unknown}` **تُتخطّى** (412-414) |
| 8 | **أي عضو آخر** ⇒ تحذير `"Unexpected archive member ignored during metadata-only import"` (193) |
| 9 | كل JSON يُقرأ بسقف 4 MiB عبر `read_limited_member` | 27 |

### 3-2. `profile_from_export_zip_import_preview` (209)

- ينشئ `instance_mode="classic"`
- ملاحظات: `"Imported metadata-only from this app's profile export zip. No files were extracted…"` (224)
- عبر `create_legacy_profile` ⇒ `legacy` / `legacy_unknown`

> **فارق مهم عن الاستيراد الكامل:** لا استخراج ولا كتابة ملفات. **هذا يحقق `foundations.md` §4.7 و`AGENTS.md` §5.6.**

---

## 4. الاستيراد من مجلد — `profile_folder_import.py`

| الخطوة | السلوك |
|---|---|
| `preview_profile_folder_import` (45) | **قراءة فقط** — يفحص `mods` · `config` · `resourcepacks` · `shaderpacks` · `saves` |
| `profile_from_folder_import_preview` (73) | ينشئ بروفايلاً مربوطاً: `linked_instance` / `user_linked` (814 في mw) |
| **الرسالة الصريحة** | «**No files were created, moved, deleted, or modified in the selected folder.**» |
| `allow_file_deletion_on_remove` | يُضبط **`False`** صراحةً (112) |
| **رفض** | `.zip` · مفقود · ليس مجلداً ⇒ `ValueError` |
| **يبدأ من `mods/`** | `_detection_root_for_selected_folder` يتصعّد مستوى (انظر `profiles.md` §2.2) |
| **يتطلب بيانات من المستخدم** | إن كان الكشف غير معروف ⇒ **يجب** أن يوفّر المستخدم إصدار ماينكرافت/اللودر |

> `test_preview_folder_import_detects_common_folders_and_builds_linked_profile_without_writes` · `test_folder_import_rejects_zip_or_non_folder_paths`

---

## 5. تصدير المودات المحفوظة — `profile_mod_export.py`

- **Markdown · CSV · نص عادي** (12 عموداً، 10-23)
- **«stored profile metadata only; no live API resolving is performed»** (93, 123)
- `export_readable_profile_mod_list` (10984)
- مجلد الأعمدة موسّع: معرّف المزوّد · الجهة · SHA-1 مثبّت · إصدارات ماينكرافت · اللودرات · التصنيفات · **عدد أثر التبعيات المطلوبة — يُحلّ علىdemand حيّاً**

---

## 6. صيغ أخرى

| الصيغة | الدالة | الملاحظات |
|---|---|---|
| **JSON** | `export_selected_profile` (12632) | حزمة `{profile, mods}` |
| **JSON (استيراد)** | `import_profile` (12687) | يتطلب كائن `profile` **و** قائمة `mods`.إن وُجد **دبّوس إصدار** ⇒ يسأل **نعم = حفظ التثبيتات / لا = أحدث متوافق** (12716-12725) |
| **`.mrpack`** | `export_selected_profile_mrpack` (12890) | **مراجع بعيدة فقط، بلا overrides** (انظر `modpacks.md` §8) |
| **`profile.json`** | `profile_metadata.py` | `PROFILE_JSON_SCHEMA = "…profile_metadata.v1"`؛ القديم `…profile.v2` ما زال يُقرأ؛ أي مخطط آخر ⇒ `ValueError`. **يرفض بايتات NUL في المسارات** |

---

## 7. العوالم — Worlds

### 7-1. المعاينة (**قراءة فقط**)

`build_profile_worlds_preview` (76):
- يعدّ أبناء `saves/` مباشرة، **يتخطّى الملفات المخفية** (99)
- **يسجّل الروابط الرمزية `status="Skipped symlink"` بلا فحصها** (101-112)
- يعدّ `level.dat` وأبناء `datapacks/` مباشرة (116-119)
-Anything غير `level.dat` ⇒ «Folder under saves»

**نصّه (136):** «**Worlds preview is read-only. No files are copied, imported, deleted, renamed, or installed.**»
`ProfileWorldsDialog` (1463) **لا أزرار فيه إطلاقاً.**

### 7-2. الاستيراد (**يكتب!**) — `build_profile_world_import_plan` (153)

> وصفها (160-165): «copy-only import plan… refuses overwrites, refuses symlinked source paths, requires an existing `saves/` folder, and blocks source trees that contain symlinks. It **does not create folders or copy files**.»

**الحواجب (183-225):**

| الحاجب |
|---|
| لا `saves` مضبوط |
| `saves` مفقود |
| **`saves` رابط رمزي** |
| المصدر مفقود / ليس مجلداً / **رابط رمزي** |
| **`level.dat` مفقود** |
| اسم الهدف فارغ أو يبدأ بنقطة |
| الهدف يحلّ خارج `saves` |
| **الهدف موجود** |
| المصدر يحتوي روابط رمزية |

**تحذيرات:** >20,000 ملف أو >5 GiB.

**التنفيذ:** `execute_profile_world_import` (244)
1. نسخ إلى `saves/.modrinth_bulk_downloader_world_import_staging/<stamp>/<name>` (263-283، التقدم **سقفه 95%** ثم 100%)
2. **إعادة التأكد** أن الهدف ما زال غائباً (285-286)
3. **`staging.rename(target)`** (287) — **نقل ذرّي واحد**
4. الإلغاء يُترك النسخة الجزئية في الخلفية للمراجعة اليدوية (269, 284)
5. كتابة `saves/.modrinth_bulk_downloader_world_imports/<name>-<stamp>.json` (15-16, 323-341)

**بوابات الواجهة:** معاينة ← Yes/No ← **كتابة `IMPORT`** (`WORLD_IMPORT_CONFIRMATION_WORD`, profile_worlds.py:17) ← نسخة أمان اختيارية **تشمل الـsaves** (6719-6729) ← `ProfileWorldImportWorker`.

> ⚠️ **إ olf: `test_world_import_blocks_existing_target_and_symlink_sources` بـ`return` صامت إذا لم تتوفّر الروابط الرمزية** — يُبلَّغ PASS والنصف الثاني غير مُختبَر. **عيب اختبار حقيقي.**

---

## 8. مقترح للنقل إلى Rust (غير مُلزِق)

| البند | القرار |
|---|---|
| صيغة التصدير | **قرّرها أنت.** Colossal Quest lob `.zip` + `profile.json` + `installed_mods.json` + `export_manifest.json`. الوسم الوحيد «غير قابل للاستيراد» هو `schema` |
| **بلا استخراج للاستيراد** | احتفظ: `ZipArchive` + قراءة JSON محدودة + لا `extract` |
| قائمة الاستبعاد | **قائمة واحدة مشتركة** بين التصدير والمعاينة — لا تكرّر |
| الرموز الحسّاسة | طابق على `split(|c| !c.is_alphanumeric())` مثل الكود القديم |
| `profile.json` | نفس `serde` للنقراءة والكتابة |
| العوالم | `fs::rename` واحد بعد نسخ — **لا تغيّرها** |
| **بصمة CurseForge** | غير موجودة — **قرار جديد لا قرار نقل** |

## 9. غير مُتحقَّق

| البند |
|---|---|
| لم أُشغّل أي اختبار |
| `app\services\profile_materialization.py` · `profile_metadata_repair.py` · `profile_runtime_artifacts.py` · `profile_workspace_summary.py` لم تُقرأ |
| حالة رمز NUL في المسارات — **مُستنتجة من اسم اختبار**، لم يُقرأ الاختبار |
| `test_world_import_blocks_existing_target_and_symlink_sources` — **`return` صامت بدل `skipTest`** |
| الدالة `export_selected_profile` JSON — لم تُقرأ |

---

## 10. تصحيحات مُقحَمة (قراءة `workers.py`)

> **المصدر:** `app\ui\workers.py` (3,328 سطراً).

### 10-1. 🔴 **استيراد المجلد واستيراد zip: لا يوجد عامل إطلاقاً**

| ما ظننتُه عاملاً | الحقيقة |
|---|---|
| `ProfileFolderImportWorker` | **غير موجود.** `preview_profile_folder_import` يُستدعى متزامناً من `main_window.py:12795` |
| `ProfileExportZipImportWorker` | **غير موجود.** `preview_profile_export_zip_import` من `main_window.py:12847` |

**غير مُتحقَّق:** هل تنفّذ هذه الخدمات قراءات قرص **على خيط الواجهة**؟ هذا خطر تجمّد، لكن لم أقرأ أجسامها.

### 10-2. **عامل التصدير موجود ويعمل بخيط**

`ProfileExportWorker` (3085-3121):
- `finished = Signal(object)` · `progress(5)` قبل · `progress(100)` بعد
- `write_profile_export_zip(plan, output_path, cancel_callback=…)` — **المُنفِّذ الوحيد**
- **إلغاء حقيقي**: يُفحص **قبل** النداء (3099-3101)، **وبعد** عودته (3112-3114)، وفي فرع الاستثناء (3118-3119)
- الرسالة تؤكّد «No final export zip was written» — **وهذا السلوك الذرّي يعيش في الخدمة، غير مُتحقَّق منه هنا**

### 10-3. **الإلغاء هنا صحيح — بخلاف التحديث**

| العامل | إلغاء حقيقي؟ |
|---|---|
| `ProfileExportWorker` | ✅ (3100/3113/3119) |
| `MrpackExportWorker` (3022-3082) | ✅ (3065) — **لكن يُفحص بعد مرحلة الحل كاملة**؛ والمشاريع الفارغة ⇒ `raise ValueError` لا `finished([])` |
| `MrpackImportWorker` (3124-3267) | ✅ خمس نقاط (3145/3163/3192/3202) — **هذا هو النمط الجيد** |

### 10-4. **`MrpackImportWorker` — إثراء اختياريrizz**

- **إن كان `manifest.blockers` غير فارغ ⇒ يُتخطَّى الإثراء بالشبكة كلياً** (3172) — **صفر طلبات**. ✅ ممتاز.
- فشل المزوّد أثناء الإثراء ⇒ **تحذير** يُضاف إلى `format_warnings` والاستيراد **ينجح** (3185-3189).
- فشل `get_project` ⇒ `ModProject` اصطناعي بعنوان `project_id` (3215-3220).
- **بلا كتابة على القرص إطلاقاً** — قراءة فقط.
- لا `progress(100)` عند النجاح.

### 10-5. **العامل الوحيد الذي يمرّر السجلّ مباشرةً للخدمات**

`MrpackInstallWorker` (3270-3328): `operation_journal: OperationJournal` **مطلوب غير اختياري**؛ ويمرّره إلى `install_mrpack_plan` و`persist_mrpack_profile` مباشرةً بدلاً من `DownloadOperationTracker`.
**ترتيب التحقق:** فحص الإلغاء **قبل** `lease.acquire()` (3294-3300) ⇒ «يُلغى» لا «مشغول».

### 10-6. **مصحّحات على ما سبق في هذه الوثيقة**

| الموضع | التصحيح |
|---|---|
| §3.3 «ينتظر `_has_running_worker()`» | ✅ صحيح — العمّال **ينتهون themselves**؛ لا انتظار في `workers.py` |
| §5 «`log_download`» | في `UpdateDownloadWorker` **غير مغلّف بـtry/except** ⇒ فشل قاعدة يُجهض الدفعة. في `UpdateInstallWorker` مغلّف. انظر `updates.md` §10-3 |
| §7.2 الاستيراد «بلا استخراج» | ✅ **مؤكَّد** — `MrpackImportWorker` لا يفكّ ولا يكتب |
| §4 «`ProfileWorldImportWorker`» | **موجود** (2483-2519) — إلغاء حقيقي + `execute_profile_world_import` |
| §7.1 معاينة العوالم | لم يوجد عامل لها — الخدمة تُستدعى مباشرة (.)