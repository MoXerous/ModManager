# screens.md — الشاشات الحالية والمجموعات المقترحة

> **المصدر:** `app\ui\main_window.py` (13,061 سطراً — **لم يُقرأ سطراً بسطر**) · استُخرجت البنية من أسماء الأصناف وأرقام الأسطر وأسماء الدوال (`grep` البنية) · `app\ui\pages\*` · `app\ui\title_bar.py`
> **⚠️ honesty:** كل ما يلي مستخرج من **البنية والPressable names**، لا من أجسام الدوال.

---

## 1. الهيكل الحالي — نافذة واحدة بأربعة أضلاع

```
WorkspaceRoot (4609)
├─ TitleBar                       app/ui/title_bar.py:26
└─ WorkspaceShell
   ├─ WorkspaceNavRail (148 px)   5301   · 5 أزرار + Back/Forward + العلامة
   └─ WorkspaceMainPages          5287   · CurrentPageStackedWidget (4469)
      ├─ HomePage                 pages/home_page.py:239
      ├─ Discover (مدمج)          4881-5269
      ├─ ProfilesPage             pages/profiles_page.py:389
      ├─ ProfileDetailsPage       pages/profile_page.py:46
      └─ DownloadManagerPage      pages/download_manager_page.py:26
```

| زر الشريط الجانبي | الدالة | السطر |
|---|---|---|
| `Home` | `show_home_workspace` | 6091 |
| `Discover` | `show_add_content_workspace` | 6105 |
| `Profiles` | `show_workspace_library` | 6133 |
| `Downloads` | `show_download_manager_workspace` | 6148 |
| `Settings` | `open_settings` | 7913 |
| `Back` (Alt+←) | `navigate_back` | 6010 |
| `Forward` (Alt+→) | `navigate_forward` | 6064 |

---

## 2. جرد الشاشات

### 2-1. `HomePage`
**يقرأ:** `repo.list_profiles` · `list_favorites` · معرّف البروفايل النشط · عدد المشاريع المحفوظة · نص الخطأ.
**يفعل:** Create Profile · Open Profile · Discover · Profiles · Operations (مركز العمليات) · Retry.
**لا يكتب شيئاً مباشرة** — يبعث إشارات فقط.

### 2-2. Discover — ثلاثة ألواح

| اللوح | المحتوى |
|---|---|
| **شريط الفلاتر** (4883) | `Clear filters` · Loaders (`loader_input` + `shader_platform_panel`) · Minecraft versions · Categories (حقول ديناميكية من `_refresh_extra_filter_controls:7053`) · Environment (عميل/خادم، لـ `mod`/`modpack` فقط) · Install target (`download_dir_input` + Browse) |
| **بطاقة البحث + النتائج** (4857, 4926) | `search_input` · `Sort by` · `Search` / `Discover` · `Load more`/`Previous`/`Next` + `Page: -` · شريط إجراءات: `Download selected` · `Pause` · `Selection ▾` · `Actions ▾` · ملخّص + حالة فارغة · `results_table` (13 عموداً) |
| **لوحة المفتش** (5092) → `right_tabs` | **`Details`** (عنوان + HTML للقراءة + `Show versions`) · **`Selected`** (قائمة، نقرة مزدوجة = إزالة) · **`Favs`** (`Add to selection` · `Save to profile`) · **`Downloads`** (جدول 5 أعمدة + Operations/History/Clear) |

**تبويبات المحتوى:** Mods · Data Packs · Resource Packs · Shaders · **Worlds** (6655).
**أسفل المقسّم:** `progress_bar` + `download_status_label` + لوحة **Activity Log** (`log_area`).

### 2-3. `ProfilesPage`
Create · Import folder · Import zip · Open · Open+Select · Manage Profiles… · بحث · ترتيب.

### 2-4. `ProfileDetailsPage` — خمسة تبويبات

| التبويب | الأزرار |
|---|---|
| **Overview** | Validate · Repair Metadata… · Open Profile Folder · Open Mods Folder |
| **Installed Content** | فلتر + بحث · `View Profile Mods` · جدول · `Selected N` · `Enable selected` · `Disable selected` · `Remove selected…` |
| **Worlds** | `View Worlds` (قراءة فقط) · `Import World…` |
| **Logs** | `Open Logs` · `Open Crash Reports` |
| **Options** | Profile Settings · Backup · Restore Backup… |

**كل الأزرار المعطّلة بلا بروفايل نشط** (render 296-307).

### 2-5. `DownloadManagerPage`
3 تبويبات: **In Progress / Completed / History** · بحث · Select All · Clear · رؤوس قابلة للنقر للترتيب.

---

## 3. حوارات — 24

| # | الصنف | السطر | العنوان |
|---|---|---|---|
| 1 | `SettingsDialog` | 1069 | `"Settings"` — 4 تبويبات |
| 2 | `ActiveProfileSelectDialog` | 1311 | `"Select active profile"` |
| 3 | `DataPackWorldTargetDialog` | 1387 | `"Choose Data Pack World Target"` |
| 4 | `ProfileWorldsDialog` | 1463 | `"Profile Worlds"` — **بلا أزرار** |
| 5 | `DownloadReportDialog` | 1527 | `"Download report"` |
| 6 | `VersionHistoryDialog` | 1585 | `f"Version history - {project.title}"` |
| 7 | `DownloadHistoryDialog` | 1756 | `"Download history"` |
| 8 | `DownloadReviewDialog` | 1893 | `"Review download"` |
| 9 | `MrpackImportPreviewDialog` | 2193 | `"Install .mrpack as new profile"` |
| 10 | `InstalledScanDialog` | 2437 | `"Installed files scan"` — **13 عموداً** |
| 11 | `UpdatePreviewDialog` | 2668 | `"Update preview"` |
| 12 | `BackupManagerDialog` | 2902 | `"Manage update backups"` |
| 13 | `ProfileEditorDialog` | 3020 | `"Create Profile"` / `"Edit profile"` |
| 14 | `ProfileInstanceDialog` | 3251 | `f"Profile as Instance - {profile.name}"` |
| 15 | `InstanceHealthDialog` | 3544 | `f"Instance Health Check - {profile.name}"` — قراءة فقط |
| 16 | `ProfileValidationDialog` | 3634 | `f"Validate Profile - {profile.name}"` — قراءة فقط |
| 17 | `ProfileExportOptionsDialog` | 3740 | `f"Export Profile - {profile.name}"` |
| 18 | `ProfileExportPreviewDialog` | 3786 | `f"Preview Profile Export - {plan.profile_name}"` |
| 19 | `ProfileExportZipImportPreviewDialog` | 3861 | `"Import Profile Export Zip"` |
| 20 | `ProfileFolderImportDialog` | 3995 | `"Import Profile From Folder"` |
| 21 | `ProfileMetadataRepairDialog` | 4141 | `"Repair Profile Metadata"` |
| 22 | `ProfileManagerDialog` | 4196 | `"Manage profiles"` |
| 23 | `PreLaunchActivationDialog` | 14141 | `"Activation required"` — **⇒ Drop** |
| 24 | `OperationCenterDialog` | `operation_center.py:179` | `"Operation Center"` |
| — | Target Version Planner (**بلا صنف**) | 11388 | `f"Target Version Planner - {profile.name}"` |
| — | Saved mods (**بلا صنف**) | 11069 | `f"Saved mods - {profile.name}"` |

> **26 نافذة لوغلاقها.**

---

## 4. التدفقات — تسع

| # | التدفق | الخطوات |
|---|---|---|
| 1 | **إنشاء بروفايل** | قيم افتراضية → محرّر → **معاينة مجلد موجود** (إلغاء/ربط) → `create_custom_profile` → `materialize_profile_workspace` → **تفعيل** → **تراجع كامل عند الفشل** (`delete_profile` + إلغاء التفعيل + مسار `PROFILES`) |
| 2 | **فتح/استيراد** | Open → حل الإصدارات المتوافقة →؛ استيراد JSON (يسأل: حفظ التثبيتات أم أحدث)؛ استيراد zip (ميتا فقط)؛ استيراد مجلد (**قراءة فقط**then ربط)؛ `.mrpack` |
| 3 | **بحث + تثبيت** | تبويب نوع → فلاتر → `Search` → تحديد صفوف → `Download selected` → **هدف صريح** (قاعدة بيانات للعالم) → **تأكيد الوجهة** → **خطة الدفعة** (إلزامي) → حوار مراجعة التبعيات → نسخة `saves` اختيارية → `DownloadWorker` → `log_download` + تحديث المانيفست + مزامنة الميتافيزقا |
| 4 | **مسح محلي + استيراد** | يفضّل Bروفايل نشطاً (وإلا يرفض) → SHA-1 مقابل `installed_mods.json` → حوار 13 عموداً → استيراد مطابق ⇒ `linked_instance`/`user_linked` · **قص اختياري للميتافيزقا فقط** |
| 5 | **فحص تحديث** | متطلّبات (مجلد + إصدار + **لودر ملموس**) → معاينة (فرق إصدار + نوع + تبعيات + نسخة) |
| 6 | **تثبيت تحديث آمن** | **إعادة تحقق** من المعرّف والهدف → عقد سلامة من 5 بنود → **Yes/No/Cancel** للنسخة → **`UPDATE_INSTALL`** → تراجعEarlier |
| 7 | **تفعيل/تعطيل** | معاينة (أول 20 + 12 محجوب) → Yes/No → تنفيذ مع **تراجع عكسي** |
| 8 | **نسخ احتياطي / استعادة** | قائمة → معاينة (مع حجب `saves`) → Yes/No → **كتابة `RESTORE`** → تنفيذ بحفظ الوجهة القائمة جانباً |
| 9 | **تصدير/استيراد** | وضع → **معاينة قائمة دقيقة الاحتواء** → كتابة ذرّية → استيراد ميتا فقط بلا استخراج |

---

## 5. التنقّل

`app/application/navigation_controller.py` — منطق **بلا Qt**، نقيّ، قابل للاختبار:

| البند | القيمة |
|---|---|
| سعة السجل | **`HISTORY_CAPACITY = 32`** لكل مكدّس |
| `WorkspaceRoute` | `(kind, profile_id=None)` مُجمَّد؛ `PROFILE_DETAILS` **يتطلب** `int` موجباً، و**يمنع** `profile_id` على غيره |
| `Push` = `navigate` | لا-op إذا تطابق المسار؛ يضيف للحاضر؛ **يمسح `forward`** |
| `Replace` = `replace` | **لا يمسّ `back`** — المسار الذي جئت منه يبقى |
| `Restore` | `commit_back`/`commit_forward` = **مقارنة وتبديل (CAS)**؛ ترفض إن لم يطابق `expected` |
| `discard_*` | تنزع المرشّح بلا تنقّل |
| الحجب | أي **modal** يمنع `navigate_back` (6011) |

> **⚠️ بمعيار `AGENTS.md` §10 استُبدل Wouter به.** 32 قيداً منطقياً **بلا قيمة** في Wouter. **Drop — بحتة.**

---

## 6. المجموعات المقترحة

> `foundations.md` §9 يحدد **ست شاشات**. الطول أدناه هو اتجاهي من تدفقات القسم 4 + عدد الحوارات.

| # | الشاشة | ما فيها | التقليص |
|---|---|---|---|
| **1** | **النسخ (Instances)** | قائمة + إنشاء + استيراد + فتح. **لا تبويبات فرعية** | −20 نافذة |
| **2** | **تفاصيل النسخة** | تبويب واحد فقط: **المحتوى المثبَّت** (جدول + شريط إجراءات). التحقق والإصلاح والنسخ والاستعادة **أوامر في شريط علوي أو قائمة سياق** | −4 تبويبات → 1 |
| **3** | **التصفح (Discover)** | شريط فلاتر + جدول + **لوحة جانبية واحدة** (تبويب التفاصيل فقط). المفضّلة/المحدد/التنزيلات **أقسام قابلة للطيّ في نفس اللوحة** | −3 تبويبات جانبية |
| **4** | **المود باكات** | استيراد/تصدير + سجلّ. **شاشة مستقلة فعلاً** — تحتاج confirmations كثيرة | +1 (كانت داخل حوارات) |
| **5** | **التنزيلات** | الطابور + السجلّ + **مركز العمليات** (علامة تنبيه، لا صفحة) | — |
| **6** | **الإعدادات** | مسارات + لغة + ثيم + مفتاح CurseForge | — |

### 6-1. قرارات تخفيض

| # | القرار | السبب |
|---|---|---|
| R-1 | **احذف شريط التنقّل الخلفي/الأمامي** | Wouter يستبدله. `HISTORY_CAPACITY` لا معنى |
| R-2 | **لا صفحة `Worlds`** | معاينة قراءة فقط + استيراد نادر ⇒ **أمر في تفاصيل النسخة** |
| R-3 | **لا صفحة `Logs`** | زر «افتح المجلد» كافٍ |
| R-4 | **مركز العمليات = نافذة** | `modeless` كما كانت |
| R-5 | **حوارات 15 و16 (InstanceHealth وProfileValidation) تُدمجان** | كلاهما **قراءة فقط بجدول صفوف** ⇒ **نافذة واحدة** `ValidateProfile` |
| R-6 | **حوار 17 و18 (خيارات ومعاينة التصدير) تُدمجان** | خطوة واحدة |
| R-7 | **حوار 6 (VersionHistory) يبقى منفصلاً** | يحتاج تصفية واختيار |
| R-8 | **حوار 23 (PreLaunchActivation) ⇒ Drop** | الترخيص |
| R-9 | **`TargetVersionPlanner` يبقى منفصلاً** | 8 أعمدة + 3 عمليات |
| R-10 | **`SavedMods` viewer ⇒ Side sheet** | قراءة فقط |
| R-11 | **حوار 3 (DataPackWorldTarget) → خطوة داخل المعاينة** | — |

### 6-2. النتيجة

| | قبل | بعد |
|---|---|---|
| صفحات | 5 + شريط تنقّل | **6** (كما في `foundations.md` §9) |
| حوارات | 26 | **~12** |
| تبويبات | 5 (تفاصيل) + 4 (جانبية) + 3 (تنزيلات) | **1 + 1 + 2** |

---

## 7. ⚠️ ما هو **غير مُتحقَّق**

| البند |
|---|
| **لم يُقرأ `main_window.py` سطراً بسطر** (13,061 سطراً). كل ما سبق من الأسماء والأسطر |
| `app\ui\pages\*` لم تُقرأ كاملة |
| ما إذا كانت بعض العناصر **مخفية كـ«widgets توافق»** (PSR: `PROJECT_MAP.md:32` يذكر ذلك صراحةً) |
| سلوك لوحة المفاتيح الدقيق واختصاراتها |
| поведение RTL — **لا يوجد i18n في المشروع القديم إطلاقاً** |
| حالة الفرز/التصفية الدقيقة في كل جدول |
| `PHASE18_23_IMPORT_FORMAT_EXPANSION_STUDY.md` — قد يقترح شاشات إضافية |

> **⚠️ انتبه:** `AGENTS.md` §5.9 يفرض **i18n من أول شاشة و RTL**. المشروع القديم **بلا i18n إطلاقاً** — كل النصوص إنجليزية حرفية في الكود. **الواجهة الجديدة ليست ترقية، بل كتابة جديدة.** |
