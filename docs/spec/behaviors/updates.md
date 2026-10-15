# behaviors/updates.md — فحص التحديث، ومعاينة التحديث، والتثبيت الآمن

> **المرجع:** `app\models.py::UpdatePreviewResult` · `app\ui\main_window.py:7309-7690` (سلاسل النداء منقولة من تقرير الوكيل، **غير مقروءة سطراً بسطر**) · `app\ui\pages\download_manager_page.py`
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
