# behaviors/journal.md - سجلّ العمليات (Operation Journal)

> **المرجع:** `app\services\operation_journal.py` (354 سطراً) · `app\operation_models.py` (434) · `app\services\operation_tracking.py` · `app\services\operation_lease.py` · `app\composition.py:83-87`
> **هذا هو القسم 4 المنقول من `storage.md`** — فُصل لأن الملف تجاوز 400 سطراً.
> **للتخطيط والتخزين:** `storage.md`.


## 1. سجلّ العمليات — `app/services/operation_journal.py`

### 4-1. المكان والاسم

- الجذر: `DATA_DIR / "operations"` (`composition.py:83,86`).
- الاسم: `{uuid}.json` بمعرّف UUID **أحرف صغيرة قانونية إجبارياً** (`_canonical_operation_id` 351-361 يُحلّل بـ`UUID(value)` ويشترط `value.lower() == str(parsed)`).
- القفل: `.active-operation.lock` — **نقطة، فلا يتصادم مع `*.json`**.
- المؤقتات: `.{operation_id}.{uuid4hex}.tmp` — نقاط، فلا يراها `discover()`.
- حد القراءة لكل سجل: `MAX_OPERATION_JOURNAL_BYTES = 4 MiB` — **على القراءة فقط**.

### 4-2. الشكل

```python
json.dumps(operation_record_to_dict(record), indent=2, sort_keys=True, ensure_ascii=False) + "\n"
```

**18 مفتاحاً علوياً** (19 مع كتلة `pack` الاختيارية — §4-2-1): `schema` · `operation_id` · `revision` · `kind` · `state` · `created_at` · `updated_at` · `profile_id` · `target_path` · `staging_path` · `backup_path` · 5 عدّادات **مشتقّة لا مخزَّنة** (`planned_count`, `completed_count`, `failed_count`, `cancelled_count`, `skipped_count`) · `message` · `items` · `pack` *(اختياري: `mrpack_install` فقط)*.

**كل عنصر 8 مفاتيح:** `item_id` · `project_id` · `title` · `filename` · `state` · `partial_path` · `final_path` · `message`.

**الأنواع:**
- `OperationKind` = `download_batch | update_download | update_install | mrpack_install`
- `OperationState` = `planned | running | completed | failed | cancelled`
- `OperationItemState` = `queued | downloading | verifying | committing | completed | skipped | failed | cancelled`

**التحليل مجموعة مغلقة:** `_reject_unknown_or_missing_keys` (444-454) ترفض **المفاتيح المجهولة والغائرة** معاً على السجلّ والعنصر. و`operation_record_from_dict` **يعيد اشتقاق العدّادات الخمسة ويتحقق منها** (385-397) ويرفع `operation counter mismatch for <key>`. ومعرّفات العناصر المكرّرة مرفوضة في `__post_init__` (165-166).

### 4-2-1. نسخة السجلّ (v2) وكتلة `pack`

السجلّ الذي يكتبه هذا التطبيق نسخته `modmanager.operation_journal.v2`. النسخة `v1` (السابقة) **تُقرأ كما هي**: لا مفتاح `pack` فيها، وهي سجلّات كتبها هذا التطبيق قبل إضافة الكتلة، فرفضها يحوّل كل سجلّ موجود على القرص إلى ملف تالف. أي نسخة غير معروفة **تُرفض** عند القراءة: السجلّ الذي كتبه بناء أحدث قد يحمل مفاتيح تُقرأ خطأً هنا، والردّ الصادق على «لا أعرف هذا الملف» هو الرفض لا التأويل.

- الكتلة `pack` **اختيارية** وتُكتب فقط لعملية `mrpack_install`: 4 مفاتيح مغلقة على الجانبين (`pack_name` · `pack_version_id` · `archive_sha512` · `archive_path`). سجلّ عملية أخرى لا يحملها، وسجلّ `mrpack_install` بلا `pack` **مرفوض**: سجلّ لا يسمّي الأرشيف لا يمكن استئنافه، إذ لا معنى لـ«استئناف ماذا».
- كل عنصر من عناصر السجلّ يسمّي `partial_path` (ملف `.part` الذي تملكه العملية) و`final_path` (الوجهة التي وصل إليها الملف). هذان المساران هما **إثبات** أن ملف `.part` على القرص لنا: لا شيء آخر يميّز ملفنا عن ملف غريب، و`safety::atomic::write_part` يرفض استبدال `.part` لا يستطيع نسبته — فلا يُحذف إلا ما يسمّيه سجلّ.
- `revision` يزيد **مرة واحدة** لكل كتابة، حتى حين تحمل الكتابة عدة انتقالات معاً (F-076): السجلّ ملف واحد، وكتابة واحدة = مراجعة واحدة.
- الكتابة الواحدة تحمل عدة انتقالات لأن السجلّ ملف واحد: كل كتابة تعيد كتابة الملف كله، والملف يكبر مع عدد العناصر. فيُكتب انتقال الملف (انتقاله إلى `downloading` ثم `verifying` ثم `committing`) في كتابة واحدة، ويسير في نفس الكتابة انتهاء الملف السابق — فتكتب مراجعة واحدة لكل ملف بدل أربع، ويتقلّص حجم ما يُكتب إلى الربع تقريباً.

### 4-3. دلالة `revision`

- `start_operation` يكتب `revision=1`، `state=PLANNED`، و`created_at == updated_at`، مع إعادة توليد UUID في حلقة `while path.exists()`.
- كل طريقة مُغيِّرة تزيد **بواحد** بالضبط: `replace_items`، `transition_operation`، `bind_profile`، `transition_item`.
- **`bind_profile` لا يزيد إن كان المعرّف نفسه** (152-153).
- `transition_operation` إلى الحالة نفسها **لا يزال يزيد ويعيد الكتابة**.
- التزامن المتفائل: `_check_revision` (338-348) — `None` يتخطى، `bool` أو غير `int` يرفع `OperationRevisionConflict("expected operation revision must be an integer.")`.
- القفل `threading.RLock` (43) يُحيط القراءة-التعديل-الكتابة كاملة. مُثبَت بـ`test_concurrent_item_transitions_do_not_lose_updates`: 24 خيطاً، المراجعة النهائية `2 + 24`.

### 4-4. الكتابة الذرّية

`_write_record_unlocked` (306-336):
1. `_ensure_root_for_write()` يرفض جذراً رابطاً رمزياً **(قبل وبعد** `mkdir`)، ويرفض غير-مجلد.
2. يرفض `target.is_symlink()` ⇒ `"operation journal record cannot replace a symlink."`
3. `temp.open("x")` — إنشاء حصري.
4. كتابة + `flush()` + `os.fsync()`.
5. `chmod 0o600` best-effort.
6. **`os.replace(temp, target)`** — ذرّي على NTFS وPOSIX.
7. `_best_effort_fsync_directory` **يعود فوراً على ويندوز**؛ على POSIX يفتح و`fsync` وكل `OSError` مُبتلَع، والـfd يُغلق في `finally`.
8. عند أي استثناء: حذف `tmp` best-effort، ثم `OperationJournalError("could not commit operation journal record: …")` مع إبقاء استثناءات `OperationJournalError` السليمة كما هي (334-335).

### 4-5. الانتقالات المسموحة

**على مستوى العملية:** `planned → {running, failed, cancelled}` · `running → {completed, failed, cancelled}` · النهايات → **مجموعة فارغة**.
**على مستوى العنصر:** `queued → {downloading, skipped, failed, cancelled}` · `downloading → {verifying, failed, cancelled}` · `verifying → {committing, failed, cancelled}` · **`committing → {completed, failed}` — بلا حافة `cancelled`**.

> **قرار تصميمي مقصود:** خطوة الالتزام (commit) **غير قابلة للإلغاء**.

الانتقالات إلى الحالة نفسها مسموحة (عودة مبكّرة في `operation_models.py:280-281`). الحُرّاس: `replace_items` في `PLANNED` فقط · `bind_profile` في `RUNNING` فقط، وإلا `ρ`:"operation is already bound to profile ID N." · `transition_item` بمعرّف مجهول ⇒ `"operation item is missing: <id>"`.

### 4-6. `discover()` — ما يفعله و**ما لا يفعله**

**يفعل:**
- يعيد نتيجة فارغة **بلا إنشاء المجلد** إن لم يكن موجوداً ولم يكن رابطاً رمزياً (212-213).
- يُبلّغ `"operation journal root is a symlink and was not inspected."` ولا يتبعه أبداً.
- يقرأ `sorted(root.glob("*.json"), key=name.casefold())` — ترتيب أبجدي حتمي.
- يتخطّى سجلّاً رابطاً رمزياً بإشعار `"journal record is a symlink and was not read."`.
- **يتحقق أن `path.name == f"{record.operation_id}.json"`**، وإلا `"journal filename does not match operation ID; expected …"`.
- **يلتقط أي استثناء لكل ملف** ويحوّله إلى `OperationDiscoveryIssue`، و`_safe_discovery_error` (371-373) يطوي المسافات ويقصّ عند 500 محرف.

**لا يفعل — كلٌّ منها غياب **مُثبَت** لا افتراض:**

| الغياب | الدليل |
|---|---|
| لا ينشئ الجذر | 212-213 |
| **لا يكتب ولا يسمّي ولا يحذف ولا يعدّل الأذونات ولا يلمس mtime** | لقطة `(bytes, st_mtime_ns)` قبل وبعد متطابقة |
| لا يحذف ولا يجمع السجلّات المنتهية | — |
| **لا يرتّب زمنياً** ولا يفلتر بالوقت | الترتيب أبجدي بالاسم فقط (أي عشوائي لأن الأسماء UUID) |
| لا يتحقق أن `target_path`/`staging_path`/`backup_path`/`partial_path`/`final_path` **موجودة** | — |
| لا يتحقق من هاشات ولا يوفّق الأدلة | `OperationRecoveryAssessment` و`OperationEvidenceStatus` **موجودان كنواع بيانات خالصة ولا يستخدمهما `operation_journal.py` إطلاقاً**؛ و`recovery_action_allowed` مكتوب `return False` (272-274) |
| لا يأخذ قفل `OperationLeaseManager` المجاور له | — |
| لا يُظهر شيئاً للمستخدم بنفسه | دالة صافية |

### 4-7. الإبلاغ عند الإقلاع

آليتان مستقلتان:

1. **العمليات المنقطعة** — `MainWindow.__init__` ينادي `_report_interrupted_operations()` (`main_window.py:4581`)، **بعد** `_build_ui()` و**قبل** `QTimer.singleShot(0, load_remote_metadata)`:
   - لكل إشعار: `"Operation journal warning ({filename}): {message}"`
   - لكل سجلّ ناقص: `"Interrupted operation detected: {kind}; updated={…}; completed={x}/{y}; target={Path(record.target_path).name or 'target unavailable'}. No automatic recovery was attempted."`
   - **يُبلّغ باسم الملف Basename فقط** — إخفاء خصوصية مقصود. الاختبار يؤكد غياب `"Private Name"` و`r"C:\Users"`.
   - الاختبار يؤكد أيضاً أن الدالة **لا تحوي** `ModrinthClient`، `_run_worker`، `unlink(`، `rmtree(`، `shutil.move`، `QMessageBox`.

2. **فشل الترحيل** — `main_window.main()` (14241-14258): `except DatabaseMigrationError` ⇒ `QMessageBox.critical(None, "Database startup failed", f"{exc}\n\nThe application did not modify profile or Minecraft files.")` ⇒ `SystemExit(2)` — **قبل الثيم، والترخيص، وبناء النافذة، والعمال، والشبكة**. خطأ غير متوقّع يُ(propagate) بلا إخفاء.

---

## 2. ملاحظات على تصميم ما يجب ألا ننسخه

>Coverage الكامل في `data-model.md`. الخلاصة هنا:

1. **`get_profile` يُسقط 11 عمود v2 بصمت** (`repositories.py:166-169`)، و`list_profiles` كذلك. أي مستهلك_driving للميتافيزقا لا يعرف أنه لا يملكها.
2. **`ON DELETE CASCADE` صامت** على المحتوى الأساسي.
3. **بلا WAL وبلا busy timeout** ⇒ تعارض فوري بين عمليتين.
4. **`AppRepository.__init__` ينفّذ الترحيلات** عند كل إنشاء — اتصال جديد + قراءة `user_version` **لكل عامل في كل عملية**.
5. **timestamps بثلاثة ترميزات غير متوافقة** في مخطط واحد.

---

## 3. غير مُتحقَّق

| البند |
|---|
| **لم أُشغّل أي اختبار** — كل ما سبق قراءة مصدر، لا تنفيذ |
| فرع POSIX الحقيقي (`os.link` + `O_DIRECTORY`) **لا يُنفَّذ على ويندوز** في الاختبارات |
| `app/services/operation_tracking.py` و`app/ui/workers.py` و`app/application/operation_center.py` لم تُقرأ كاملة — تأكيداتها من ملف الاختبار لا من التنفيذ |
| `docs/superpowers/specs/2026-07-13-database-migration-runner-design.md:371` مقروء عبر grep فقط |
