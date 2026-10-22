# behaviors/updates.md — فحص التحديث، ومعاينة التحديث، والتثبيت الآمن

> **المرجع:** `D:\latest_project\` (تطبيق بايثون/PySide6، `master` @ `4f71f18`) · `app\models.py::UpdatePreviewResult` · `app\ui\main_window.py:7309-7690` (سلاسل النداء منقولة من تقرير الوكيل، **غير مقروءة سطراً بسطر**) · `app\ui\pages\download_manager_page.py`
> ⚠️ **تنبيه الدقة:** `main_window.py` فيه 13,061 سطراً. ما يلي مستخرج من **أسماء الدوال ونصوص الحوارات**، لا من أجسامها. كل ما يعتمد على منطق داخلي يُوسم **غير مُتحقَّق**.

---

## 1. نموذج المعاينة — `UpdatePreviewResult` (frozen، 21 حقلاً)

| المجموعة | الحقول |
|---|---|
| الحالي | `current_version_id` · `current_version_number` · `current_file_name` · `current_sha1` · `current_size` |
| الأحدث | `latest_version_id` · `latest_version_number` · `latest_file_name` · `latest_sha1` · `latest_size` |
| النوع | `current_release_type` · `latest_release_type` |
| التبعيات الجديدة | `new_required_dependencies` · `new_optional_dependencies` · `new_embedded_dependencies` · `new_incompatible_dependencies` |

> **نموذج جيد:** «قبل» و«بعد» في نفس السجل، فجدول المعاينة لا يحتاج دمجاً في العرض.

---

## 2. فحص التحديث — `check_updates_preview` (7309)

### 2-1. المتطلّبات المسبقة (تُرفض قبل أي شبكة)

| # | الشرط | الدليل |
|---|---|---|
| 1 | مجلد موجود | |
| 2 | إصدار ماينكرافت محدّد | |
| 3 | **لودر ملموس** — يُرفض `"any"` صراحةً | 7319 |

> نفس المنطق يظهر في `ModrinthApiError`: `"Choose a specific loader/platform before checking updates."` · `"Choose a specific Minecraft version before checking updates."`
> **هذا يحترم `resolver.py:107-110`: لا تسقط من Fabric إلى Forge عند التحديث.**

### 2-2. المسار

`check_updates_preview` (7309) → `UpdatePreviewWorker` → `on_update_preview_finished` (7346) → يسجّل `«files changed: 0»` إن لم يتغيّر شيء → `UpdatePreviewDialog` (2668).

### 2-3. содержимое حوار المعاينة

`UpdatePreviewDialog` يعرض لكل مود:
فرق الإصدار (`old → new`) · نوع الإصدار (`release`/`beta`/`alpha`) · **معاينة التبعيات** · نص السلامة · **خطة النسخ الاحتياطي**.

**الأزرار:** `select_all_updates` (2867) · `request_download` (2887) · `request_install` (2894).

---

## 3. التثبيت الآمن — `install_selected_updates` (7474)

### 3-1. إعادة التحقق الإلزامي

```python
_validated_update_preview_context(...)      # 7364
```

**يعيد التحقق** أن **معرّف البروفايل النشط** و**مجلد الهدف** لم يتغيّرا منذ المعاينة — وإلا **يحجب**.

> **هذا يحقق `AGENTS.md` §5.4 (خطة ثم تنفيذ) عبر حدود زمنية**: الخطة صارت قديمة بين العرض والتأكيد.

### 3-2. عقد السلامة المعروض في التأكيد (7501)

النص يسرد خمس ضمانات:

1. الملفات القديمة ⇒ نسخة احتياطية بختم زمني
2. **لا حذف دائم لأي شيء**
3. الملفات غير المعروفة تبقى دون مساس
4. **ملف تغيّر منذ المعاينة ⇒ يُتخطّى ذلك التحديث**
5. الملفات الموجودة غير المرتبطة **لا تُكتب فوقها**

### 3-3. ثلاثية النسخ الاحتياطي — `_confirm_profile_update_backup_before_update` (7534)

**`Yes / No / Cancel`** — وليس ثنائياً.

| الإجابة | المسار |
|---|---|
| `Yes` | `_start_profile_update_backup_before_install` (7558) → `ProfileUpdateBackupWorker` → `on_profile_update_backup_finished` (7589) → **ينتظر `_has_running_worker()`** ثم `_start_update_install_worker` (7633) |
| `No` | يذهب مباشرة |
| `Cancel` | يتوقف |

> **انتظار انتهاء العامل قبل بدء التالي** — نمط تسلسل صحيح. في Rust: `await handle.join()` لا «ابدأ واتركه».

### 3-4. التحديث بالتنزيل فقط — `download_selected_updates` (7403)

ينزّل إلى **مجلد منفصل** `<base>/modrinth-update-downloads` (7443) و**لا يمسّ أي ملف قائم إطلاقاً**.

---

## 4. مخطّط الإصدارات المستهدفة — Target Version Planner

| البند | التفصيل |
|---|---|
| المشغّل | `open_profile_target_version_planner` (11379) — حوار **EPHEMeral بلا صنف مسمّى** |
| المتطلّبات | بروفايل محفوظ **بـ** مودات محفوظة |
| الضوابط | إصدار ماينكرافت (11402) · اللودر `fabric/forge/neoforge/quilt/vanilla` (11412) · `Check` (11417) |
| العامل | `ProfileUpgradePreviewWorker` (11522) |
| الجدول | 8 أعمدة: `Title, Current pinned, Current file, Target version, Target file, Status, Notes` |
| `Suggest common target` | 11418 → `ProfileCommonTargetWorker` |
| `Download selected target versions` | 11419 — **بعد** الفحص فقط |
| **نص المقدمة** | «**This preview does not change files.**» |

**ما يفعله المخطّط:**
- يقارن المودات المحفوظة بإصدار/لودر **مختار**
- يقترح أهدافاً مشتركة **صارمة** و**أفضل** (`PHASE16_19_3`, `PHASE16_19_5`, `PHASE16_19_7`)
- ينزّل إصدارات الهدف المختارة
- **يحفظ نسخاً احتياطية من الملفات المعروفة غير المتوافقة**
- **يسجّل الملفات المحلية غير المعروفة ويُبقيها دون مساس** أثناء تنظيف الهدف
- **يزامن `game_version`/`loader` في البروفايل** بعد اكتمال التنزيلات

> **مُثبَت:** `test_target_cleanup_logs_unknown_local_files_kept` (تأكيد نص مصدر) · `PHASE16_19_40_SAFE_UPDATE_INSTALL_SAVED_MODS_SYNC_REPORT.md`
> **كاش محلي:** `_downloadable_versions_cache` و`_candidate_resolution_cache` **داخل العامل فقط** (`PHASE16_19_17`) — انظر `lessons.md` §6.

---

## 5. التنزيلات المجمَّعة والتقدّم

| البند | التفصيل |
|---|---|
| التوازي | `SETTING_PARALLEL_DOWNLOADS` افتراضياً **3**، محصور 1–8 |
| المعاملات | `SETTING_DOWNLOAD_RETRIES` افتراضياً **1**، محصور 0–3 ⇒ **محاولتان** |
| لوحة التحميل | 5 أعمدة: `Project / File / Status / Progress / Message` |
| ترويسات | `Operations` · `History` · `Clear` |
| قائمة عارضة | `show_downloads_context_menu` (13354) · `open_selected_download_file_location` (13390) |
| الإيقاف المؤقت | `toggle_download_pause` (8449) |
| الإلغاء | `cancel_current_operation` (8471) |

> تفاصيل آلة الحالات في `downloads.md` §1. **الأهم:** كل الأطوار تمرّ بـ`state_callback` قبل العمل المقابل، و`operation_tracking.py:152` **يرفض أي طور غير معروف** — **fail-closed**.

---

## 6. سجلّ التنزيلات وواجهته

`app/services/download_manager_service.py` (26 سطراً) يملك استعلامات `download_history`: **فلترة بالحالة** · **بحث** · **ترتيب بقائمة بيضاء**.

| البند | الدليل |
|---|---|
| `ORDER BY {sort_by} {order}, id {order} LIMIT ?` | `repositories.py` |
| `safe_limit = max(1, min(int(limit), 1000))` | 552 |
| حتى **أربعة** تنبيهات `LIKE '%…%'` ببداية حرف بدل | 569 |
| قائمة الترتيب المسموحة | `ALLOWED_DOWNLOAD_HISTORY_SORT_COLUMNS` (18-35)، مفحوصة عند 555-556 |
| `clear_download_history` | `DELETE FROM download_history` بلا معاملات |

> ✅ **النمط الجيد:** القائمة البيضاء لعمود الترتيب **حماية حقيقية من حقن SQL** — **انقلها**. (`sqlx::query_as` + `QueryBuilder` يصلحان الأمر تلقائياً، لكن الحارس البديل لا يضر.)
> ⚠️ **العيب:** القائمة تكرّر `schema_contract.CURRENT_COLUMNS["download_history"]` — يمكن أن تنحرف بصمت.

---

## 7. مركز العمليات — قراءة فقط

| البند | التفصيل |
|---|---|
| الحوار | `OperationCenterDialog` (`app/ui/operation_center.py:179`) — **modeless**، `"Operation Center"` |
| الأوامر | `Assess local evidence` (276) · `Mark as abandoned` (283) · `Refresh` (294) |
| **ما لا يفعله** | **لا Resume · Retry · Rollback · Restore · Delete · Move · Cleanup · Acknowledge · terminalize** |
| razones | `assets/` + `app/operation_models.py:272-274` |

### 7-1. التقييم — `Assess local evidence`

- يعيد تحميل **هذا المعرّف فقط**، ويرفض انحراف المراجعة/الحالة
- **سقفان**: **512 مساراً فريداً** · **128 سلفاً معجمياً**
- **تمرّتان للميتافيزقا فقط**، يرفض الروابط الرمزية ونقاط إعادة التوجيه وأنواع الأخطاء واستبدالات الملفات
- يُعيد **أعداداً وفئات مجمّدة بلا مسارات**
- `recovery_action_allowed` = **`return False` دائماً**
- `OperationCenterDialog` يمسح النتائج القديمة عند تغيير التحديد/الفلاتر/اللقطة

**دلالات الأدلة** (`OperationEvidenceStatus`): `NONE_PRESENT` · `PARTIAL` · `FINAL` · `MIXED` · `NO_PATHS` · `UNSAFE` · `LIMIT_REACHED`

> **إن كانت الأدلة `MIXED`** ⇒ `cannot be automatically determined` — **يرفض التخمين بدل أن يخمّن.**
> **تعزيز على ويندوز:** `test_windows_reparse_attribute_is_unsafe_without_path_is_junction` ينادي `_is_link_or_junction` خاصة ⇒ reparse-point attribute `0x400` على ويندوز ⇒ **UNSAFE**.

---

## 8. اقتراح للنقل إلى Rust (غير مُلزِم)

| البند | القرار |
|---|---|
| `UpdatePreviewResult` | `#[derive(TS)]` مع `before`/`after` منمّقتين |
| إعادة التحقق قبل التنفيذ | **نفسها**: `let ctx = revalidate(&ticket)?;` |
| Yes/No/Cancel للنسخ الاحتياطي | `enum BackupChoice { Yes, No, Cancel }` |
| «الملف تغيّر منذ المعاينة ⇒ تخطَّ» | `(path, size, mtime)` في التذكرة، وأعد المقارنة عند التنفيذ |
| «فحص التحديث» (بادئة) |مطابق لـ`version_files/update` — **`foundations.md` §4.4 ينصح بتجميع التحديثات دفعة واحدة** — اجعله `update_check_batch(project_ids, hashes)` |
| القوائم ذات القوائم البيضاء | `enum HistorySort` + `QueryBuilder` |
| مركز العمليات | **`Assessment` فقط، `recovery_action_allowed: false` دائماً** — نفس العقد |

## 9. غير مُتحقَّق

| البند |
|---|---|
| **لم يُقرأ `app/ui/workers.py`** (3,035 سطراً) — أجسام `UpdatePreviewWorker` · `UpdateDownloadWorker` · `UpdateInstallWorker` · `BackupScanWorker` · `BackupRestoreWorker` · `ProfileUpgradePreviewWorker` · `ProfileCommonTargetWorker` **غير موصوفة** |
| منطق «المقارنة الصرامة» و«الهدف الأفضل» في `ProfileCommonTargetWorker` |
| نصوص الحوارات منقولة كما وردت في تقرير الوكيل |
| `tests\qa\test_profile_pages.py` و`test_home_page.py` (اسم فقط، أجسامها لم تُقرأ) |

---

## 10. تصحيحات مُقحَمة (قراءة `workers.py` — 3,328 سطراً لا 3,035)

> **المصدر:** `app\ui\workers.py` + `app\ui\task_signal_proxy.py` + `app\application\task_controller.py`.
> **هذه التصحيحات تُلغي أو تُكمّل ما سبق.** كل سطر يحمل رقمه في المصدر.

### 10-1. 🔴 **الإلغاء ليس `cancelled` في عمّال التحديث — بل `finished` ناجح**

| العامل | كيف يُبلَّغ الإلغاء فعلاً |
|---|---|
| `UpdatePreviewWorker` | `finished([])` عند إلغاء مبكّر (1944) · **`finished(results)` بنتائج جزئية** (1965) |
| `UpdateDownloadWorker` | `finished([DownloadResult(…,"cancelled",…)])` (2112/2199/2250) — **بلا `cancelled.emit` إطلاقاً** |
| `UpdateInstallWorker` | صف `cancelled` اصطناعي واحد (2650) ثم `finished(results)` العادي (2700) |
| `BackupScanWorker` | `break` ثم **`finished(partial_entries)`** (2312→2337) — **لا يمكن تمييزه عن مسح قصير** |
| `BackupRestoreWorker` | صف `"cancelled"` ثم `progress(100)` **حتى بعد الإلغاء** ثم `finished` (2366/2396) |
| `ProfileUpgradePreviewWorker` | صف `{"status":"cancelled"}` **داخل** `finished(rows)` (357) |
| `ProfileCommonTargetWorker` | `finished({"suggested":"", "message":"…cancelled."})` (980) |

**العامل الوحيد في هذه المجموعة الذي يستخدم `cancelled` الحقيقي: لا شيء.**
مقابل ذلك **صحيح**: `ScanInstalledWorker` (1770/1781/1787) · `MrpackImportWorker` (3145/3163/3192/3202) · `MrpackInstallWorker` (3295/3323) · `ProfileUpdateBackupWorker` (2417/2437) · `ProfileBackupRestoreWorker` · `ProfileWorldImportWorker` · `ProfileExportWorker` · `MrpackExportWorker`.

> **يؤكّد قرارك في F-036:** `DownloadManager` يُبلّغ الإلغاء كـ`status` داخل حمولة ناجحة. **اجعله `IpcError::Cancelled` مستقلاً في Rust.**
> **تعارض هنا مع §3 أعلاه:** قلتُ إن «الإلغاء حالة لا استثناء» — **صحيح على مستوى `DownloadManager`، وخاطئ على مستوى العمال.** أعلى طبقة تسطّحها.

### 10-2. 🔴 **الكود القديم يسمح بـ`http` في روابط `.mrpack`**

`mrpack_service.py` كان يقبل `http` و`https`؛ `modpacks.md` §3.5 يذكر `HTTPS` فقط. **غير مُتحقَّق أيّهما ساري الآن** — لم أقرأ `mrpack_service.py:386` بعد.

### 10-3. **`UpdateInstallWorker` — تفاصيل لم تكن موصوفة**

| البند | السطر | السلوك |
|---|---|---|
| **ختم الأدلة في `__init__` لا `run()`** | 2593-2595 | `backup_dir` و`staging_dir` يُحسبان **عند إنشاء العامل** ⇒ إعادة تشغيل العامل تشارك الختم |
| **القفل قبل التحقق** | 2603-2606 | `lease.acquire()` **قبل** فحص `mods_dir` |
| **`tracker.start` قبل أول تعديل** | 2616-2637 | ✅ **الترتيب صحيح** — يسبق `staging_dir.mkdir` (2638) |
| **حارس TOCTOU** | 2736-2746 | **يعيد حساب هاش الملف القديم** ويقارنه بـ`item.sha1` وقت المعاينة ⇒ «Local file changed since preview» |
| **الملف موجود مسبقاً** | 2806-2838 | إن وُجد الهدف وكان **هاشه مطابقاً**: يحفظ القديم، **يحذف المنزَّل من الخلفية** (2818-2819)، ويبلّغ `success` **دون كتابة الهدف إطلاقاً** (2822-2828). وإلا ⇒ `failed` «Nothing was replaced» (2833-2838) |
| **`mark_committing` قبل `shutil.move`** | 2842-2846 → 2854 | ✅ صحيح |
| **⚠ التراجع قد يهبط باسم مختلف** | 2878-2888 | `_backup_old_file` و`_restore_backup` يستخدمان `unique_path` ⇒ التراجع قد ينتج **`name-1.jar`** |
| `repo.log_download` في `try/except` | 2671-2690 | هنا مغلّف. **في `UpdateDownloadWorker` (2231-2242) ليس مغلّفاً** ⇒ فشل قاعدة يُجهض الدفعة كاملة |

### 10-4. **🐛 `UpdateDownloadWorker` يترك سجلّ العملية مفتوحاً عند التحديد الفارغ**

`return` عند 2148 **دون `tracker.finish()`** ⇒ سجلّ العملية يبقى `RUNNING` إلى الأبد ⇒ `journal.md` §1.6 سيبلّغه عند كل إقلاع تالٍ كعملية مقاطعة.
**هذا علّة حقيقية في القديم، لا شرط سلوك.**

### 10-5. **`UpdatePreviewWorker` لا يرى المودات المعطّلة**

| العامل | مصدر الملفات | النتيجة |
|---|---|---|
| `UpdatePreviewWorker` | `folder.iterdir()` + suffixes `{".jar",".mrpack",".zip"}` (1927-1930) | **لا ملفات `.jar.disabled` ولا `disabled_mods/`** |
| `ScanInstalledWorker` | `iter_local_mod_scan_files(folder)` (1749) | Top level **+** `disabled_mods/` (local_mod_classification.py:53-57) |

**نفس المفهوم — «امسح مجلد mods» — بمجموعتين مختلفتين من الملفات.** مؤكَّد-behaviour، لا شرط.

### 10-6. **معاينة التحديث تتجاهل التثبيتات المحفوظة**

`ProfileUpgradePreviewWorker` يمرّر **`strip_pins=True`** (340-350، موثّق في 310-313) ⇒ **المعاينة تقارن أحدث لا المثبَّت.**
`ProfileCommonTargetWorker` و`MrpackExportWorker` لا تفعلان ذلك (`MrpackExportWorker` `allow_alpha=False` افتراضياً، خلافاً لـ`ProfileUpgradePreviewWorker`).

### 10-7. **«الاقتراح الصارم» ليس التقاطَع الخام**

`ProfileCommonTargetWorker` (679-1178):
1. تقاطَع جارٍ لـ`game_versions` عبر المشاريع (1007) — المشاريع بلا إصدارات قابلة للتنزيل تُستبعد (993-996).
2. **ثم** كل مرشّح **يُتحقَّق منه بالمبدّل ضد كل المشاريع** (837-865، 1031-1035، 1070) ⇒ **أول مرشَّح يتحقَّق هو الاقتراح الصارم.**
3. `best_upgrade` استشاري فقط، **يتطلب مشروعين على الأقل** (882-883)، ولا يُتحقَّق بالمبدّل إلا **أعلى 16 مرشّحاً** (919).
4. **قائمة أهداف مُثبَّتة** `"1.21.11"`…"`1.20"` (1042-1046) + أولوية خاصة لـ`1.20.1` (1078-1082) — **منطق UX داخل العامل، لا في خدمة.**
5. يصل إلى `resolver._candidate_loaders` **الخاصة** (744، مع `# noqa: SLF001`).
6. **بلا إشارة `progress`** — وبها ثلاث أشكال مختلفة لـ«فارغ» (1010-1017 / 1168-1176 / الإلغاء) ⇒ المستهلك يجب أن يقرأ `suggested`/`message` لا الفراغ.

### 10-8. **آلية رفض الاستدعاءات القديمة —Global واحدة لا لكل عامل**

`main_window.py:8325-8332`: `thread.quit` مربوط بـ**الإشارات الخام** (فالخيط يتوقف دائماً)، لكن **البوابة على المسار الوسيط فقط** عبر `TaskSignalProxy._dispatch` → `TaskController.accept_terminal` (113-120)، وهي **خانة `_active_task` عالمية واحدة**.
**⇒ أول إشارة نهائية تفوز؛ كل ما بعدها يُهمَل صامتاً.** وهذا بالضبط ما يمنع عرض نتائج بحث ملغى.

### 10-9. **مسارات تعديل بلا سجلّ عمليات**

| العامل | السطر | التعديل بلا journal |
|---|---|---|
| `BackupRestoreWorker` | 2380 | `shutil.copy2` |
| `ProfileUpdateBackupWorker` | — | `copytree`/`copy2` في الخدمة |
| `ProfileBackupRestoreWorker` | — | استعادة |
| `ProfileWorldImportWorker` | — | `staging.rename(target)` |
| `ProfileExportWorker` · `MrpackExportWorker` | — | كتابة zip |

**فقط عاملان يعدّلان ملفات تحت سجلّ: `UpdateDownloadWorker` و`UpdateInstallWorker`.** وفيهما الترتيب صحيح (§10-3).
**انقل هذا الترتيب للجميع أو اقبل أن انقطاع العملية يترك أثراً بلا أثر.**

### 10-10. **تمّ التصدير أم لا**

| الصنف | حقيقة |
|---|---|
| `ProfileFolderImportWorker` | **غير موجود** — استدعاء خدمة متزامن في `main_window.py:12795` |
| `ProfileExportZipImportWorker` | **غير موجود** — `main_window.py:12847` |
| `ProfileExportWorker` | **موجود** (3085-3121) · `write_profile_export_zip` · إلغاء حقيقي |
| `MrpackExportWorker` | **موجود** (3022-3082) · **بلا مشاريع ⇒ `raise ValueError`** لا `finished([])` |
| `MrpackImportWorker` | **موجود** (3124-3267) · **حواجب ⇒ يُتخطَّى الإثراء بالشبكة كلياً** (3172) · فشل المزوّد ⇒ **تحذير لا فشل** (3185-3189) |
| `ProfileValidationWorker` | **موجود** (2522-2550) · **لا يفحص `cancel_requested` إطلاقاً** — فقط يوجّه `RuntimeError` |
