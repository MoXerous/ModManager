# behaviors/modpacks.md — `.mrpack` و CurseForge zip

> **المرجع:** `app\services\mrpack_service.py` (730 سطراً) · `app\services\mrpack_installer.py` (1,659) · `tests\qa\test_mrpack_installer.py` (1,489)
> ⚠️ **CurseForge zip: غير موجود إطلاقاً.** لا كود ولا اختبار. `foundations.md` §4.7 و§4.4 يصفانه كتصميم **مستقبلي**. كل ما يخصّه هنا **غير مُتحقَّق**.

---

## 1. الصيغ المدعومة

| الصيغة | الحالة في الكود القديم |
|---|---|
| `.mrpack` (Modrinth) | ✅ **مُنفَّذ معماريّاً** (قراءة + خطة + تثبيت + تصدير) |
| CurseForge `.zip` (`manifest.json`) | ❌ **غير موجود** |
| packwiz | ❌ مؤجَّل في `foundations.md` §4.7 |

---

## 2. قراءة الفهرس — `read_mrpack_manifest(path)`

### 2-1. الأنواع (كلها frozen، والمجموعات `MappingProxyType`)

`MrpackFileEntry(path, hashes, downloads, file_size=0, env={}, warning="", client_requirement="required")`
`MrpackOverrideEntry(archive_member, destination, layer, file_size)`
`MrpackManifest` · `MrpackMatchedFile` · `MrpackExportSummary` · `MrpackImportPreview`

> ⚠️ **`MrpackExportSummary` مُجمَّد لكنه يحمل `warnings: list[str]` قابلة للتعديل** — ثغرة تجميد سطحية.

### 2-2. خط أنابيب التحليل — **فتح zip واحد فقط** (مُثبَت باختبار)

| # | الخطوة | الرفض |
|---|---|---|
| 1 | **تحقق صارم** من المسار: مفقود / ليس ملفاً / **رابط رمزي** | `ValueError("Choose a valid regular .mrpack file that is not a symlink.")` |
| 2 | فتح + بثّ **SHA-256** للأرشيف كله | ⇒ `archive_sha256` |
| 3 | `inspect_zip_archive(archive)` | `SafeArchiveError` (انظر `archive-safety.md`) |
| 4 | **يجب** يحتوي `modrinth.index.json` | `ValueError("This archive does not contain modrinth.index.json.")` |
| 5 | `read_limited_member(..., max_bytes=MRPACK_INDEX_MAX_BYTES)` | حدّ القراءة |
| 6 | UTF-8/JSON صالح **و** يجب أن يكون كائناً | `ValueError("modrinth.index.json must contain valid UTF-8 JSON.")` / `"…must be a JSON object."` |
| 7 | `_manifest_override_entries(inventory)` | الطبقات + تحذيرات الأعضاء |
| 8 | `manifest_sha256 = sha256(index_bytes)` | **هاش عنصر الفهرس وحده** |

`MRPACK_INDEX_MAX_BYTES = 4 MiB`

### 2-3. الصيغة المدعومة

| الحقل | المطلوب |
|---|---|
| `formatVersion` | **`1` فقط** — غير ذلك **حاجب** |
| `game` | **`"minecraft"` فقط** — غير ذلك **حاجب** |

**طبقات الـoverrides المعترف بها:** `overrides/` · `client-overrides/` · `server-overrides/`.
أي عضو آخر غير `modrinth.index.json` ⇒ **تحذير** (`test_unexpected_members_are_warnings_not_override_files`).

`VALID_ENV_VALUES = {"required","optional","unsupported","unknown"}`
`MRPACK_DOWNLOAD_HOSTS = {cdn.modrinth.com, github.com, raw.githubusercontent.com, gitlab.com}` — **مضيف مطابق تماماً، casefolded، بلا نطاقات فرعية.**

### 2-4. استخراج اللودر — `_manifest_loader`

```python
_LOADER_DEPENDENCIES = {fabric-loader→fabric, forge→forge, neoforge→neoforge,
                        quilt-loader→quilt, quilt→quilt}
```
- بلا مفتاح لودر ⇒ `("vanilla", "", ())`.
- **أكثر من زوج (لودر، إصدار) واحد** ⇒ حاجب `"Pack declares multiple mod loaders: {names}."` ويُعيد `("", "", …)`.
- `test_vanilla_pack_has_explicit_loader_and_empty_loader_version` يثبّت حالة الفانيلا.

### 2-5. التحقق الصارم لكل ملف — `_strict_manifest_files`

**حواجب، لكن العناصر تُعاد رغم ذلك** (لا فقدان صامت):

| الحاجب |
|---|
| مدخل غير قاموس |
| مسار غير آمن (مطلق / محرك / `..` / فارغ) |
| تكرار/تصادم NFC+casefold في الوجهة |
| `hashes` ليس كائناً |
| `sha1` لا يطابق `^[0-9a-f]{40}$` |
| `sha512` لا يطابق `^[0-9a-f]{128}$` |
| `fileSize`/`file_size` ليس `int` موجباً |
| لا رابط `https://` ذي netloc |
| `env` ليس كائناً، أو `env.client ∉ {required, optional, unsupported}` |

---

## 3. خطة التثبيت — `build_mrpack_install_plan(manifest, request, *, matched_files=())`

### 3-1. حقول الخطة — `MrpackInstallPlan` (frozen)

`request` · `manifest_sha256` · `pack_version_id` · `game_version` · `loader` · `loader_version` ·
`final_profile_root` · `staging_root` · `source_archive_sha256` · `remote_items` · `skipped_remote_items` ·
`override_items` · `skipped_server_overrides` · `issues` · `remote_bytes` · `override_bytes` ·
`content_bytes` · `required_disk_bytes` · `free_disk_bytes`

مستق deriv: `blockers` (شدة `error`) · `warnings` · `is_installable = not blockers`.

**`staging_root` = `parent/.{name}.mrpack-staging-{uuid4().hex}`**
**`required_disk_bytes` = `content_bytes + MRPACK_INSTALL_METADATA_RESERVE`** (1 MiB)

### 3-2. ترتيب التنفيذ (مهمّ: **الترتيب يحدّد الأولوية**)

1. `_validate_source`
2. `_validate_target`
3. **إعادة إخراج** حواجب/تحذيرات `manifest` كرموز `error/"manifest_blocker"` و`warning/"manifest_warning"`
4. تطبيع الاختيار الاختياري
5. خريطة الملفات المطابقة
6. **حلقة لكل ملف**
7. التحقق من الاختيار الاختياري
8. `overrides` **ثم** `client-overrides`
9. `server-overrides` (تُسجَّل كمتخطّاة)
10. **فحص القرص** (الأخير)

> **فحص القرص أخيراً وليس أولاً** — قرار تصميمي. في Rust: فكّر إن كان يجب نقله.

### 3-3. قاعدة الاختيار

| `env.client` | السلوك | `selection_reason` |
|---|---|---|
| `required` | **دائماً** | `"required"` |
| `optional` | فقط إن كان مفتاح التصادم NFC-casefold في `selected_optional_paths` | `"selected_optional"` |
| `unsupported` | **أبداً** | `"client_unsupported"` |
| أي قيمة أخرى | `error/"client_environment_invalid"` | — |

### 3-4. أولوية الـoverrides — **المدقّق**

`_register_destination` يسمح **بالاستبدال للمسار المطابق تماماً** في الأزواج المرتّبة فقط:

```
remote → overrides        ✔
remote → client-overrides ✔
overrides → client-overrides ✔
```

**أي شيء آخر** ⇒ `error/"destination_collision"`.
**مسار هو بادئة لآخر** ⇒ `error/"destination_parent_collision"`.

> **الأثر العملي:** `overrides/` ثم `client-overrides/` **يتغلّبان** على الملفات البعيدة وعلى `overrides/` الأساسية؛ **`server-overrides/` يُتخطّى دائماً** مع `info/"server_overrides_skipped"`.

**تخطّي المحتوى الخاص بالخادم:**
- ملفات بعيدة بـ`client: "unsupported"` ⇒ `skipped_remote_items` + `info/"remote_file_skipped"`
- كل `server-overrides/` ⇒ `skipped_server_overrides` + `info/"server_overrides_skipped"`
- **سلامة الرابط للعناصر المتخطّاة لا تُفحص إطلاقاً** (تُستدعى `_allowed_urls(selected=False)` التي لا تُصدر شيئاً).
> `test_unselected_optional_and_unsupported_urls_do_not_block_client_plan` يثبّت هذا صراحةً.

### 3-5. سياسة روابط التنزيل البعيد

`_allowed_urls` يبقي فقط من-survivors لـ`validate_mrpack_download_url` (HTTPS + مضيف مسموح تماماً + بلا بيانات اعتماد)، مع إزالة تكرار.

| الحالة | الرمز | الشدّة |
|---|---|---|
| ملف **مختار** ورابط مرفوض | `download_url_rejected` | `warning` |
| ملف **مختار** ولا ناجين | `download_url_missing` | `error` |
| ملف **متخطّى** ورابط مرفوض | *(صمت)* | — |

**`test_accepts_only_exact_official_https_https_hosts`:** يُقبل 4، ويُرفض 7 — `http://` · userinfo · `cdn.modrinth.com.evil.invalid` · **`sub.cdn.modrinth.com` (نطاق فرعي!)** · `localhost` · `127.0.0.1` · `example.com`.

### 3-6. الوجهات المحجوزة وغير الآمنة

| الرمز | القاعدة |
|---|---|
| `reserved_destination` | أي مكوّن `casefold` إلى `profile.json` أو `installed_mods.json`، **أو** يبدأ بـ`.modrinth_bulk_downloader` |
| `unsafe_destination` | محارف غير آمنة `< > " \| ? *` في أي مكوّن، أو مسارات خطرة |
| `target_name_invalid` | اسم البروفايل نفسه: اجتياز/مطلق/محرف غير آمن |

### 3-7. جدول رموز كامل

| الشدّة | الرموز |
|---|---|
| **`error`** (26) | `manifest_blocker` · `source_invalid` · `source_mismatch` · `target_parent_missing` · `target_not_direct_child` · `target_name_invalid` · `target_exists` · `staging_exists` · `client_environment_invalid` · `reserved_destination` · `optional_selection_invalid` · `matched_file_invalid` · `matched_file_duplicate` · `unsafe_destination` · `download_url_missing` · `file_size_invalid` · `sha1_invalid` · `sha512_invalid` · `override_layer_invalid` · `override_size_invalid` · `destination_collision` · `destination_parent_collision` · `insufficient_disk` · `disk_check_failed` |
| **`warning`** (2) | `manifest_warning` · `download_url_rejected` |
| **`info`** (2) | `remote_file_skipped` · `server_overrides_skipped` |

> **26 رمز خطأ مُسمّى — لا نص حر.** هذا **نموذج ممتاز** لـ`IpcError { code }` في `AGENTS.md` §5.3. **انقل هذا التسلسل كما هو.**

---

## 4. التثبيت — `install_mrpack_plan(plan, ...)`

### 4-1. البوابات قبل أي كتابة

| البوابة | الرسالة |
|---|---|
| خطة غير قابلة للتثبيت | `"Cannot install a blocked .mrpack plan."` |
| خطة ملغاة مسبقاً | — |
| **`_revalidate_plan_source_and_roots`**: يُعيد قراءة الحزمة، ويعيد فحص هاش الفهرس وهاش الأرشيف والحواجب والمسارات | `"The .mrpack changed or became invalid after preview."` |

### 4-2. التسلسل

1. بدء سجلّ عملية (`MRPACK_INSTALL` · `RUNNING`)
2. بناء `MrpackInstallRollbackContext`
3. إنشاء `staging_root` بصلاحية `0o700`
4. كتابة **إيصال `planned`**
5. لكل عنصر بعيد:
   - **تجربة كل رابط بالترتيب** (احتكاك)
   - `_ensure_staging_parent`
   - `path_guard` يرفض مسارات الخلفية الرمزية أو الهاربة
   - `url_validator=validate_mrpack_download_url`
   - استدعاء طور تصاعدي رتيب: `downloading→verifying→committing` **لا يتراجع**
   - التأكيد أن **الوجهة المخططة بالضبط** موجودة كملف عادي غير رمزي
   - `verify_artifact(...)` بالحجم + SHA-1 + SHA-512
   - فشل ⇒ `expected.unlink(missing_ok=True)` **وتجربة الرابط التالي**
   - **أي حالة خارج `{success, skipped}` تُجهض التثبيت كله**
6. استخراج الـoverrides: إعادة تحقق من `info.file_size == item.file_size`، بثّ إلى `.{name}.{uuid}.tmp`، `flush()`+`os.fsync`، فحص الطول المنسوخ، `os.replace(temp, target)`، `finally: temp.unlink()`
7. **بعد كل الـoverrides يُعاد حساب SHA-256 للأرشيف كله** ⇒ `"The .mrpack changed after preview."`
8. `_materialize_standard_profile_directories`: `mods, config, resourcepacks, shaderpacks, saves`
9. `_write_provisional_profile_state`: `profile.json` (**`profile_id = None`**) + `installed_mods.json`
10. `_receipt_matches(required_state="planned")` ⇒ `_revalidate_publish_paths` ⇒ **`atomic_publish_directory_noreplace`**

### 4-3. النشر بلا استبدال

| المنصّة | الآلية |
|---|---|
| **Windows** | `os.rename` (يفشل إن الوجود موجود) |
| **Linux** | `ctypes` → `renameat2(AT_FDCWD, src, AT_FDCWD, dst, RENAME_NOREPLACE=1)` |
| غير ذلك | يرفع |

شروط مسبقة: شقيقان إلزاميان · الخلفية مجلد عادي غير رمزي · **الوجهة غير موجودة** (`"Final profile root appeared before publication."`).

> ⚠️ **`renameat2` غير متاح في كل أنظمة لينكس** (يحتاج `fs` يدعم `RENAME_NOREPLACE`). `test_linux_publish_uses_renameat2_noreplace` يحقن libc وهمية — **لا يختبر المنصّة الحقيقية**.

### 4-4. التراجع

`rollback_mrpack_install` — `shutil.rmtree` **فقط** على مجلد:
- إيصاله يطابق بايت-ببايت `{schema:"…mrpack_install.v1", operation_id, final_root_basename, manifest_sha256, state:"planned"}` — **مجموعة المفاتيح يجب أن تطابق تماماً**
- حجمه ≤ 64 KiB، ملف عادي غير رمزي
- وأبوه = `final_root.parent`

`test_rollback_refuses_missing_or_mismatched_marker` يثبت رفض الإيصال الناقص **و** المرفوض.

---

## 5. الاستمرارية — `persist_mrpack_profile(plan, filesystem, *, profile_manager, repository, journal, timestamp=None)`

### 5-1. التحقق من السياق (`_validate_persistence_context`)

- جذور نظام الملفات + هاش الفهرس **يجب أن يساوي** الخطة
- الجذر المنشور ما زال يحمل إيصال `planned`
- سجلّ العمليات: `MRPACK_INSTALL` · `RUNNING` · `target_path` مطابق · `staging_path` مطابق · `profile_id is None`

### 5-2. الخطوات

| # | خطوة |
|---|---|
| 1 | `create_linked_modpack(profile, source_pack=SourcePackIdentity(provider="local_mrpack", project_id=manifest_sha256, version_id=pack_version_id, manifest_sha256=…))` |
| 2 | `journal.bind_profile(...)` (مع فحص المراجعة) |
| 3 | `repository.add_mods_to_profile(profile_id, managed_projects)` |
| 4 | **إعادة كتابة `profile.json` بالمعرّف الحقيقي** + إعادة كتابة `installed_mods.json` |
| 5 | `_write_receipt_atomic(..., state="committed")` |
| 6 | على **أي** فشل: `repository.delete_profile(profile_id)`؛ فإن نجح، تراجع أيضاً عن الجذر المملوك؛ **وإن فشلت إزالة الصف فالجذر يُحفظ عمداً** ⇒ `"profile root preserved for recovery because the profile row still exists"` + السجلّ `FAILED` |
| 7 | فشل الانتقال النهائي `COMPLETED` **لا يتراجع** — يعيد `journal_warning` مع بروفايل مُثبَّت |

### 5-3. الدليل المُثبَت

`test_every_precommit_persistence_failure_rolls_back_new_row_and_owned_root` — subTests على `profile_create` · `journal_bind` · `profile_mod_save` · `profile_json` · `installed_manifest` · `receipt_commit`. **كل واحد** يترك `list_profiles() == []` ولا جذر منشور وسجلّ `FAILED`.

`test_profile_row_rollback_failure_preserves_owned_root_for_recovery` — عند فشل التراجع عن الصف، **الجذر يبقى** والإيصال `"planned"`.

`test_terminal_journal_failure_preserves_committed_profile_and_root` — البروفايل **يبقى مُثبَّتاً** والإيصال `"committed"` لكن السجلّ ما زال `RUNNING`.

---

## 6. ما يُكتب في `profile.json`

`PROFILE_JSON_SCHEMA = "modrinth_bulk_downloader.profile_metadata.v1"`
(الشكل القديم `modrinth_bulk_downloader.profile.v2` ما زال يُقرأ؛ أي مخطط آخر ⇒ `ValueError`.)

```json
{
  "schema": "modrinth_bulk_downloader.profile_metadata.v1",
  "profile": {
    "id": null,
    "name": "…",
    "minecraft_version": "…",
    "modloader_type": "…",
    "modloader_version": "…",
    "profile_type": "linked_modpack",
    "profile_status": "unknown",
    "paths": { "profile_root": "…", "minecraft_root": "…", "mods": "…",
               "config": "…", "resourcepacks": "…", "shaderpacks": "…", "saves": "…" },
    "created_at": "…", "updated_at": "…", "last_used_at": "…"
  }
}
```

**البروفايل المنشأ** (`_profile_from_plan`): `instance_mode="instance"` · `instance_root == minecraft_root == profile_root` · `download_dir = mods_dir = root/"mods"` · `shaderpacks_dir` · `resourcepacks_dir` · `saves_dir` · `servers_file=root/"servers.dat"` · `config_dir` · `screenshots_dir` · `detected_game_version` · `detected_loader` · `instance_validation_status="unknown"` · الرسالة `"Profile content installed; launcher runtime not validated."` · **`allow_file_deletion_on_remove=True`** · ملاحظات `"Installed from a local Modrinth .mrpack. Minecraft and modloader runtime installation are not managed by this operation."`

---

## 7. `installed_mods.json` — الدليل المُثبَت فقط

**شرطثلاثي حتى يصبح ملف بعيد «مداراً»:**

```python
match.provider.casefold() == "modrinth"
and non-empty (project_id, version_id, sha1)
and match.sha1.casefold() == item.sha1.casefold()
and project_id not already recorded
```

السجلّ: `file_id = version_id` · `source="modrinth"` · `managed_status="managed"` · `path` = المسار نسبةً للحزمة.
`ContentProject` المقابل يُنشأ بكل حقول `preferred_*` ممتلئة.

> `test_persists_only_matched_evidence_and_commits_cross_resource_state` — **الفهرس المؤقت قبل الالتزام ليس فيه إلا المشاريع المطابقة.**

---

## 8. التصدير — `export_resolved_to_mrpack`

**يتخطّى (بتحذير)** أي مشروع: بلا إصدار · بلا ملف أساسي · **بلا SHA1 أو SHA512** · بلا رابط صالح · بلا حجم · أو نوع مشروع خارج الخريطة:

| نوع المشروع | المجلد |
|---|---|
| `mod` | `mods` |
| `resourcepack` | `resourcepacks` |
| `shader` | `shaderpacks` |
| `datapack` | `datapacks` |
| `plugin` | `plugins` |

**يكتب `modrinth.index.json` فقط — بلا overrides.**
`versionId = datetime.now(timezone.utc).strftime("%Y.%m.%d-%H%M%S")`
مفتاح اللودر: `{fabric→fabric-loader, quilt→quilt-loader, forge→forge, neoforge→neoforge}` بقيمة `"*"`.

> **ملاحظة أمنية:** التصدير بلا overrides يعني **فقد كل إعداد محلي** (config، خيارات، تعديلات). **قرار مستقبلك: هل نصدر overrides؟**

---

## 9. الاختبارات (31)

### 9-1. `MrpackManifestTests` (5)
`test_reads_identity_loader_environment_and_override_layers_once` (**فتح zip واحد** مذكور حرفياً) · `test_manifest_and_nested_collections_are_deeply_immutable` · `test_reports_strict_index_and_remote_file_blockers_without_trusting_entries` (**9 فئات حواجب**) · `test_vanilla_pack_has_explicit_loader_and_empty_loader_version` · `test_unexpected_members_are_warnings_not_override_files`

### 9-2. `MrpackDownloadUrlPolicyTests` (1)
`test_accepts_only_exact_official_https_hosts`

### 9-3. `MrpackInstallPlanTests` (8) — **كلها تأكيدات عدم تغيّر**
كل واحدة تتحقق أن **بايتات المصدر لم تتغير**، وأن **قائمة المجلد الأب لم تتغير**، وأن `final_profile_root` و`staging_root` **ما زالتا غائبتين**.

### 9-4. `MrpackFilesystemInstallTests` (10) — الأهمّ
| الاختبار | ما يحميه |
|---|---|
| `test_installs_fallback_downloads_overrides_and_publishes_with_receipt` | مرآة تفشل ثم تنجح؛ الأسبقية الكاملة |
| `test_download_cancellation_cleans_only_marker_owned_staging` | `MrpackInstallCancelled`، السجلّ `CANCELLED` |
| **`test_target_race_is_not_overwritten_or_deleted`** | **الوجهة التي تُنشأ أثناء التنزيل لا تُكتب فوقها ولا تُحذف** |
| `test_same_size_override_tampering_after_preview_is_rejected` | **تلاعب بنفس الطول** (لا يتغيّر الحجم!) ⇒ `"changed after preview"` |
| `test_remote_download_symlink_substitution_cannot_escape_staging` | ⚠ `SkipTest` بلا صلاحيات |
| `test_journal_or_receipt_failure_never_publishes` | فشل السجلّ **أو** الإيصال ⇒ لا نشر |
| `test_rollback_refuses_missing_or_mismatched_marker` | **التراجع يرفض بلا إيصال مطابق** |
| `test_windows_publish_uses_non_replace_rename` | يحقن `sys.platform` — **لا يختبر المنصّة** |
| `test_linux_publish_uses_renameat2_noreplace` | libc وهمية |
| `test_override_crc_failure_and_publish_failure_clean_owned_stage` | تنظيف الخلفية المملوكة |

### 9-5. `MrpackProfilePersistenceTests` (5) — SQLite حقيقي
انظر §5.3.

### 9-6. `MrpackInstallWorkerAndUiContractTests` (3)
⚠ `test_static_ui_contract_uses_install_preview_and_no_legacy_profile_creation` — **تأكيدات نص مصدر** على `main_window.py`/`workers.py` بتقطيع حرفي. ** هشّ أمام أي إعادة هيكلة.**

---

## 10. ما نتعلّمه ونقله إلى Rust

| البند | الحالة | ملاحظة |
|---|---|---|
| **رموز مشاكل مسمّاة** (26 خطأ / 2 تحذير / 2 info) | ✅ **انقلها حرفياً** | نموذج ممتاز لـ`IpcError { code }` |
| `renameat2` بـ`RENAME_NOREPLACE` | ⚠ يحتاج `rustix` أو `libc` | **افحص صفحة crates.io أولاً** (`AGENTS.md` §6) |
| وصف `.mrpack` | ⚠ **غير مُتحقَّق** | §12: لا مواصفة في `gap-analysis`؛ المصدر الوحيد هو `mrpack_service.py` |
| **CurseForge zip** | ❌ **جديد تماماً** | `manifest.json` بـ`projectID`/`fileID` — **لا مرجع** |
| عدم فحص روابط العناصر المتخطّاة | ✅ نقل | صحيح: لا نُجهّز قائمة بيضاء لما لن نزّله |
| `MrpackExportSummary.warnings` قابل للتعديل رغم `frozen` | ⚠ ثغرة | `Vec<String>` يحلّها في Rust |

---

## 11. ملاحظات تحليل الفجوات القديم

> قراءات `docs\modrinth-app-gap-analysis.md` عن `.mrpack` وCurseForge **مُنقولة إلى [`gap-analysis-notes.md`](./gap-analysis-notes.md)** (بما فيها: لا مواصفة صيغة في الوثيقة، وتعارضها الداخلي، وشرح ادّعاء `curseforge_client.py`).

## 12. غير مُتحقَّق

| البند |
|---|---|
| لم أُشغّل أي اختبار |
| **CurseForge zip** — صفر كود |
| `renameat2` على نظام لينكس حقيقي بلا `RENAME_NOREPLACE` |
| سلوك استعادة الاستئناف (`.mrpack` كبير) — لا يوجد |
| `docs/modrinth-app-gap-analysis.md` (97 KB) لم يُقرأ — قد يحتوي بحثاً في `.mrpack` |
| الحقول الخمسة لـ`tests\qa\test_mrpack_installer.py` و`MrpackProfilePersistenceTests` آخر الملف (غير مقروءة بالمWorker) |
