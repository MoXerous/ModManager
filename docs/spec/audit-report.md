# التقرير الختامي للتدقيق الشامل — القديم ↔ الجديد

> BASE: `4f71f18` · HEAD: راجِع سجلّ git لهذا الفرع (إسناد بيانات التدقيق في `headCommit` أدناه) · الدفتر: 276 وحدة.

> المصادر: `docs/spec/audit-ledger.json` (الدفتر)، `docs/spec/audit-findings.md` (مولَّد حتمي)،
> `.opencode/analyses/audit/audit-review.md` (المراجعة العدائية، 23 حقل `reviewNote`)،
> `.opencode/analyses/audit/defects.md` (سجلّ العيوب). القديم للقراءة فقط عند `D:\latest_project @ 4f71f18`؛
> لم تُجرَ إصلاحاتٌ على مصادر التطبيق في هذه الشريحة.

> الإسناد: `headCommit` في `audit-ledger.json` و`audit-findings.md` المولَّد يسجّل التزامَ إتمام بيانات
> التدقيق (`2fdfcdc` — آخر إصلاح لمراجعة Task 8)؛ والالتزامات التوثيقية اللاحقة (`8768f68`، `c90c743`،
> وهذا الإصلاح) لا تغيّر نطاق التدقيق ولا حصيلة الوحدات.

## ما تحقّقتُ منه

الجدول من `docs/spec/audit-findings.md`، وهو مطابقٌ لحصيلة الدفتر: **276** وحدة — 170 `match`،
73 `documented-deviation`، **26** `undocumented-deviation` (findings)، و7 `unverifiable`.

| البُعد | الوحدات | مطابق | انحراف موثّق | findings | غير مُتحقَّق |
|---|---|---|---|---|---|
| behavior | 181 | 112 | 49 | 20 | 0 |
| cli | 6 | 5 | 1 | 0 | 0 |
| data | 12 | 10 | 2 | 0 | 0 |
| deps | 4 | 3 | 1 | 0 | 0 |
| i18n | 4 | 4 | 0 | 0 | 0 |
| lessons | 10 | 9 | 1 | 0 | 0 |
| packaging | 8 | 0 | 8 | 0 | 0 |
| performance | 7 | 0 | 0 | 0 | 7 |
| safety | 10 | 10 | 0 | 0 | 0 |
| tests | 8 | 8 | 0 | 0 | 0 |
| ui | 26 | 9 | 11 | 6 | 0 |

طرق التحقّق المستخدمة في الدفتر: `code` و`test` و`browser` و`run` و`registry`. نُفِّذت المراجعة العدائية
لكل `undocumented-deviation`؛ أسفرت عن 23 حقل `reviewNote`: تخفيض `A-0215` من `high` إلى `medium`،
وإعادة تصنيف `A-0083` و`A-0128` و`A-0195` إلى `documented-deviation`، ورفض مقترحات إعادة تصنيف أخرى
بعد التحقّق من `old_ref`/`new_ref`. كل الـ26 findings النهائية `medium`؛ لا `blocker` ولا `high` بينها،
لأن مولّد التقرير يرفض `low` لوحدة `undocumented-deviation` (انظر ملاحظات المراجعة أدناه).

## Findings مرتّبة بالخطورة

الترتيب: `blocker` ثم `high` ثم `medium` حسب `severity`، وداخل الشدة حسب المعرّف. جميع الوحدات أدناه
`medium`، ومعها عيبٌ منفصلٌ ذو شدة `high` هو `D-001` في سجلّ العيوب (ليس وحدة تدقيق).

### A-0001 — F-001 — medium — method `code`

- الأثر: البحث يرسل الآن ثلاث مجموعات `facets` فقط (`project_type`، `versions`، `categories`) ويثبّت `index=relevance`؛ فلاتر التصنيف و`client_side`/`server_side` والترتيب المختار غائبة من الطبقة كلها حتى `SearchQueryDto`. البحث بالاسم والصفحات مطابقان.
- old_ref: `app/api/modrinth_client.py::search_projects` · new_ref: `core/src/providers/modrinth.rs::ModrinthClient::facets`
- التوصية: توسيع `SearchQuery` والحقل المقابل في `src-tauri/src/dto.rs` بـ`category` و`client_side` و`server_side` و`index`، وإرسالها إلى `facets()`/`search_inner` مع اختبارات لكل مجموعة.
- الشريحة المقترحة: M2 — مزوّد Modrinth (البحث والفلاتر).

### A-0045 — F-045 — medium — method `code`

- الأثر: مفتاح التصادم يستخدم `to_lowercase()` ثم NFC بدل `casefold()`؛ أسماء مثل `ß`/`ﬁ` لا تتصادم مع صيغتها الموسّعة. حالة `A.txt` مقابل `a.TXT` ما زالت مكتشفة.
- old_ref: `app/services/safe_archive.py::_member_collision_key` · new_ref: `core/src/safety/inspect.rs::collision_key`
- التوصية: إكمال تغطية Unicode `casefold` في `collision_key` مع اختبارات للحالتين sharp-s و`fi` ligature، وتوثيق أي استثناء.
- الشريحة المقترحة: M2 — أمان الأرشيف (توحيد مفاتيح الأعضاء).

### A-0054 — F-054 — medium — method `code`

- الأثر: نموذج `review_candidates` يُنتج صفوف التبعيات الاختيارية غير المحدّدة، لكن مسار الخطة الإنتاجي يتبع حواف `required` فقط، فلا تظهر صفوف اختيارية ولا مربّعات اختيار للمستخدم.
- old_ref: `app/ui/main_window.py::DownloadReviewDialog` · new_ref: `core/src/resolver/graph/mod.rs::review_candidates`
- التوصية: وصل `review_candidates` بمسار `batch.rs` وإظهار الصفوف الاختيارية في حوار المراجعة مع حفظ اختيار المستخدم في الخطة.
- الشريحة المقترحة: M2 — حلّ التبعيات + حوار مراجعة التنزيل.

### A-0055 — F-055 — medium — method `code`

- الأثر: قاعدة سحب سلسلة `required` عند اختيار تبعية اختيارية موجودة في النموذج (`requires_parent`) لكن `selected_optional` لا يُملأ من أي مسار إنتاجي، فلا تُسحب السلسلة فعليًا.
- old_ref: `app/services/resolver.py::collect_required_dependencies_for_optional_dependencies` · new_ref: `core/src/resolver/graph/mod.rs::review_candidates`
- التوصية: تمرير اختيارات المستخدم من الواجهة إلى `review_candidates` ودمج النتيجة في خطة التثبيت النهائية.
- الشريحة المقترحة: M2 — حلّ التبعيات + حوار مراجعة التنزيل (مع A-0054).

### A-0056 — F-056 — medium — method `code`

- الأثر: ترتيب التثبيت (id ← SHA-1 ← رقم الإصدار) مطابق، لكن الجديد يضيف بوابة `alpha`: التثبيت على إصدار alpha يُرفض حين `allow_alpha=false` (وهو ما يمرّره `batch.rs:59`) ويُستبدل بأحدث إصدار مستقرّ بلا إشعار للمستخدم.
- old_ref: `app/services/resolver.py::_preferred_version_if_compatible` · new_ref: `core/src/resolver/resolve/mod.rs::resolve_project`
- التوصية: الحفاظ على التثبيت على alpha أو إظهار تحذير صريح عند الاستبدال، مع اختبار يغطي حالة `allow_alpha=false`.
- الشريحة المقترحة: M2 — المحلّل واستيراد ملفات التثبيت.

### A-0062 — F-062 — medium — method `code`

- الأثر: حوار مراجعة التبعيات فقد عمود «Include»: يعرض إدخالات الخطة للقراءة فقط، بلا صفوف اختيارية ولا إمكانية إدخال/استبعاد قبل التأكيد.
- old_ref: `app/ui/main_window.py::DownloadReviewDialog` · new_ref: `src/app/pages/InstallPlanDialog.tsx`
- التوصية: إضافة مربّعات اختيار وصفوف اختيارية إلى `InstallPlanDialog` وربطها بحقول الخطة القادمة من `core`.
- الشريحة المقترحة: M2 — واجهة مراجعة خطة التثبيت (مع A-0054/A-0055).

### A-0063 — F-063 — medium — method `test`

- الأثر: معاينة dry-run مطابقة للإدخالات والإزالة المكرّرة والعدّاد والتحذيرات، لكن شرط التأكيد الخامس (`backup_matching_project_files`) مفقود؛ التعليق في المصدر يقول إنه ينتظر صاحب حالة النسخ الاحتياطي الفعلي، و`feature-parity.json` لا يسجّل الفجوة.
- old_ref: `app/services/batch_install_plan.py::build_batch_install_plan` · new_ref: `core/src/resolver/plan/mod.rs::build_batch_install_plan`
- التوصية: تمرير علم النسخ الاحتياطي إلى ملخّص الخطة وإدخاله في `requires_confirmation` مع اختبار Truth-table سادس.
- الشريحة المقترحة: M2 — خطة التثبيت الجافة/النسخ الاحتياطي.

### A-0067 — F-067 — medium — method `code`

- الأثر: تحذير «External/unknown file selected…» غير الحاجز مفقود من `PreviewEntry` التي تحمل `allowed + reason` فقط؛ قواعد إعادة التسمية والتراجع بالترتيب العكسي مطابقة ولا حذفَ للملفات.
- old_ref: `app/services/profile_content_toggle.py` · new_ref: `core/src/instances/toggle/mod.rs::preview_toggle`
- التوصية: إضافة حقل تحذير غير حاجز إلى `PreviewEntry` وعرضه في واجهة التفعيل/التعطيل.
- الشريحة المقترحة: M2 — تفعيل/تعطيل المودات.

### A-0072 — F-072 — medium — method `code`

- الأثر: خطة الحزمة القديمة تحمل قائمة مسائل برموز آلية و`severity` ومسار؛ الجديدة لا تحمل قائمة مسائل إطلاقًا وتُسقط كل رفض إلى `IpcError` واحد برمز `invalidInput`، فيختفي العرض المسبق لكل مسألة ورموزها.
- old_ref: `app/services/mrpack_installer.py::MrpackInstallIssue` · new_ref: `src-tauri/src/commands/pack.rs::pack_error`
- التوصية: إضافة حقل `issues` إلى `ModpackPlanDto` يحمل الرمز والشدة والمسار، وعرضه في معاينة الحزمة مع الاحتفاظ برسالة واضحة.
- الشريحة المقترحة: M2/M3 — تثبيت حزم `.mrpack` (واجهة المعاينة).

### A-0085 — F-085 — medium — method `test`

- الأثر: `clean_string` الجديد يحذف مقطع `${...}` داخل القيمة، بينما القديم يُفرغ القيمة فقط إن كانت placeholder بالكامل؛ قيمة مختلطة تظهر بنص مختلف. الحقول الأربعة وحدودها و`depends.minecraft` مطابقة.
- old_ref: `app/services/local_mod_metadata.py::_read_fabric_metadata` · new_ref: `core/src/jar_reader/mod.rs::parse_fabric`
- التوصية: محاكاة القديم: إفراغ القيمة إن كانت placeholder كاملة وإبقاء المقطع المضمّن كما هو، مع اختبار للحالتين.
- الشريحة المقترحة: M2 — قارئ بيانات الـjar المحلية (`fabric.mod.json`).

### A-0086 — F-086 — medium — method `test`

- الأثر: القارئ الجديد يشترط أن يكون `mods[0].modId` نصًّا ويتجاهل عُقد `dependencies` المعرّفة كـdict؛ ملف `mods.toml` مقروء عند القديم ينتج عنه لا بيانات عند الجديد. الحقول و`versionRange` الخام مطابقة للحالات السليمة.
- old_ref: `app/services/local_mod_metadata.py::_read_forge_like_metadata` · new_ref: `core/src/jar_reader/mod.rs::parse_forge_toml`
- التوصية: قبول `modId` المفقود وعُقد dict مع اختبارات توافق، دون تغيير نطاق الإصدار الخام.
- الشريحة المقترحة: M2 — قارئ بيانات الـjar المحلية (`mods.toml`).

### A-0092 — F-092 — medium — method `code`

- الأثر: القديم يقرأ `mmc-pack.json` و`modrinth.index.json` و`launcher_profiles.json` ويبني درجة ثقة (`confirmed`/`likely`/`conflict`/`unknown`) مع إصدار اللعبة/Loader؛ الجديد يكتشف الـloaders من أسماء الـjar فقط ولا يقرأ بيانات الإقلاع، فيفقد المثيل هويته المؤكَّدة.
- old_ref: `app/services/instance_detector.py::_detect_metadata` · new_ref: `src-tauri/src/commands.rs::scan_instance_at`
- التوصية: تنفيذ قرّاء بيانات الإقلاع الثلاثة وسلّم الثقة، وتمرير النتيجة إلى `detect_loaders`.
- الشريحة المقترحة: M2 — اكتشاف المثيلات/بيانات الإقلاع.

### A-0101 — F-101 — medium — method `code`

- الأثر: مرآة `profile.json` تُكتب عند الإنشاء فقط وعند إعادة الإنشاء في الاستعادة؛ لا يوجد أمر تحديث/إعادة تسمية، ومسارات تطبيق الحزمة والنسخ الاحتياطي والاستعادة لا تُحدّثها، فتصبح المرآة قديمة، ويغيب مسار التحديث (`update path`) المذكور في المواصفة.
- old_ref: `app/services/profile_metadata.py::write_profile_json` · new_ref: `core/src/instances/metadata.rs::write_mirror`
- التوصية: استدعاء `write_mirror` بعد كل تعديل على المثيل وإضافة أمر تحديث/إعادة تسمية إذا لزم.
- الشريحة المقترحة: M2 — إدارة المثيلات/مرآة البيانات.

### A-0102 — F-102 — medium — method `code`

- الأثر: القديم ينشئ خمسة مجلدات قياسية (`mods`, `config`, `resourcepacks`, `shaderpacks`, `saves`) مع `profile.json` و`installed_mods.json`؛ الجديد ينشئ `mods` و`config` فقط، ولا كاتب لـ`installed_mods.json` على هذا المسار.
- old_ref: `app/services/profile_materialization.py::materialize_profile_workspace` · new_ref: `core/src/instances/registry.rs::plan_create`
- التوصية: إضافة المجلدات الثلاثة إلى `STANDARD_DIRS` أو توثيق قرار إلغائها، وتحديد مصدر `installed_mods.json` البديل.
- الشريحة المقترحة: M2 — إنشاء المثيلات.

### A-0106 — F-106 — medium — method `code`

- الأثر: القديم يكشف مجلدًا موجودًا ويقدّم «Link Existing / Create Here Anyway / Cancel»؛ الجديد يرفض أي مجلد غير فارغ، ولا كشف ولا خيار «Create Here Anyway»، فيُمنع المستخدم من إضافة `profile.json` إلى مثيل قائم.
- old_ref: `app/ui/main_window.py::_choose_existing_create_target_mode` · new_ref: `core/src/instances/registry.rs::plan_create`
- التوصية: إضافة خطوة كشف قبل الإنشاء وخيار صريح للمتابعة في المجلد الحالي مع بقاء وضع الربط متاحًا.
- الشريحة المقترحة: M2 — إنشاء/ربط المثيلات (واجهة + `core`).

### A-0120 — F-120 — medium — method `code`

- الأثر: معاينة التحديثات القديمة (17 عمودًا) تعرض فروق التبعيات (مطلوبة/اختيارية/مضمّنة/غير متوافقة) وسبب التوافق وملاحظة الملفات وخطة النسخ الاحتياطي لكل صف؛ الجديد يعرض الملف وأحدث رقم إصدار وحالة من أربع قيم فقط.
- old_ref: `app/ui/main_window.py::UpdatePreviewDialog` · new_ref: `core/src/updates/mod.rs::check_updates`
- التوصية: إضافة حقول فروق التبعيات وسبب التوافق وخطة النسخ الاحتياطي إلى `UpdateCandidate` وعرضها في `InstanceDetails`/`UpdateInstallDialog`.
- الشريحة المقترحة: M3 — التحديثات (المعاينة وخطة التثبيت).

### A-0129 — F-129 — medium — method `code`

- الأثر: مسح المجلد المثبَّت مقابل الفهرس واستيراد المشاريع المعروفة إلى قائمة البروفايل مفقود؛ `scan_mods` يصنّف ملفات القرص فقط بلا تعرّف على مزوّد ولا إجراء استيراد.
- old_ref: `app/ui/main_window.py::import_installed_scan_matches_to_profile` · new_ref: `core/src/instances/mod.rs::scan_mods`
- التوصية: إضافة مطابقة مع الفهرس وجدول نتائج وإجراء استيراد إلى قائمة المودات المحفوظة، أو توثيق قرار إلغاء صريح (يتعارض مع Keep في `features.md`/`feature-parity.md`).
- الشريحة المقترحة: M2/M3 — المحتوى المثبَّت والاستيراد (مرتبط بـA-0203).

### A-0144 — F-144 — medium — method `code`

- الأثر: القديم يقفل حصريًا ملف `.active-operation.lock` على مستوى النظام، فيمنع نسخة ثانية من تشغيل عملية متعارضة ويعيد `Busy`؛ الجديد يستخدم `Mutex` داخل العملية فقط، فتستطيع نسختان تطبيق الحزمة نفسِه على المثيل ذاته في آنٍ واحد.
- old_ref: `app/services/operation_lease.py::OperationLeaseManager` · new_ref: `src-tauri/src/commands/pack.rs::apply_lock_for`
- التوصية: إضافة قفل ملف فعلي على مستوى النظام مع ربط أخطاء `EACCES`/`EAGAIN`/WinError بـ`Busy`، مع اختبار تعدد العمليات.
- الشريحة المقترحة: M2 — قفل العمليات/التزامن.

### A-0151 — F-151 — medium — method `code`

- الأثر: القديم يحفظ 25 إعدادًا في جدول `settings`؛ الجديد يحفظ أربعة أعلام واجهة في `localStorage` فقط، فلا إعدادات للتنزيل المتوازي ولا عدد المحاولات ولا حد البحث ولا سياق البحث الأخير. المصدر يحمل تعليق `shortcut`.
- old_ref: `app/storage/repositories.py::set_setting` · new_ref: `src/app/preferences.ts::Preferences`
- التوصية: إنشاء مخزن إعدادات في `core` للمفاتيح المطلوبة وعرضها في `SettingsPage` مع اختبارات حفظ/استرجاع.
- الشريحة المقترحة: M2 — الإعدادات (`core` + الواجهة).

### A-0170 — F-170 — medium — method `code`

- الأثر: الثلاثية `system/light/dark` مطابقة، لكن اختيار لون التيّم (accent) مفقود: القديم يبني لوحة الألوان من اسم اللون، والجديد لوحة ثابتة بلا مُنتقٍ ولا إعداد. `feature-parity.json` يسجّل الثلاثية فقط.
- old_ref: `app/ui/main_window.py::apply_app_theme` · new_ref: `src/app/preferences.ts::THEMES`
- التوصية: إضافة حقل `accent` إلى `Preferences` ومُنتقٍ في `SettingsPage` مع تطبيق على tokenات Tailwind.
- الشريحة المقترحة: M2 — المظهر/الإعدادات.

### A-0198 — screens.md §3 #5 DownloadReportDialog — medium — method `browser`

- الأثر: لا يوجد أي سطح لتقرير التنزيل: صفحة `Downloads` ثلاثة تبويبات وحقل بحث ومسح للسجل فقط، بلا إنشاء تقرير أو تصدير أو ملخّص لكل دفعة؛ ولا قرار `R` يسجّل الإلغاء.
- old_ref: `D:\latest_project\app\ui\main_window.py:1527` · new_ref: `src/app/pages/DownloadsPage.tsx:59`
- التوصية: إضافة إنشاء/تصدير تقرير التنزيلات، أو تسجيل قرار `Drop` في `decision-sheet.md` و`feature-parity.json`.
- الشريحة المقترحة: M2 — صفحة التنزيلات (واجهة).

### A-0203 — screens.md §3 #10 InstalledScanDialog — medium — method `browser`

- الأثر: جدول القديم ذو 13 عمودًا (Keep في `features.md`/`feature-parity.md`) صار قائمة مضغوطة؛ ستة أعمدة بلا سطح جديد: `Version` و`Project ID` و`Game versions` و`Loaders` و`Matched file` و`Message` — أول اثنين غائبان عن الصف وآخر أربعة غائبة من الـDTO.
- old_ref: `D:\latest_project\app\ui\main_window.py:2502` · new_ref: `src/app/pages/InstanceDetails.tsx:197`
- التوصية: إضافة الحقول الستة إلى `LocalFactDto` وعرضها في الصف، أو تصحيح ادّعاء `feature-parity.md:198`؛ لا تغيير على `feature-parity.md` في هذه الشريحة.
- الشريحة المقترحة: M2 — المحتوى المثبَّت (DTO + واجهة، مع A-0129).

### A-0206 — screens.md §3 #13 ProfileEditorDialog — medium — method `browser`

- الأثر: إنشاء البروفايل صار إنشاء مثيل بالحقول نفسها، لكن وضع «تعديل بروفايل» (الاسم/الإصدار/Loader/المجلد/الملاحظات) مفقود؛ لا أمر تحديث للمثيل ولا مسار لتصحيح الإصدار/الـloader المكتشفَين حتى في وضع الربط.
- old_ref: `D:\latest_project\app\ui\main_window.py:3036` · new_ref: `src/app/pages/NewInstanceDialog.tsx:43`
- التوصية: إضافة أمر `update_instance` في `core`/Tauri ووضع تعديل في واجهة المثيل.
- الشريحة المقترحة: M2 — إدارة المثيلات (تعديل المثيل القائم).

### A-0207 — screens.md §3 #14 ProfileInstanceDialog — medium — method `browser`

- الأثر: تصدير البروفايل كـ«Profile as Instance» مفقود؛ الجديد يتعامل مع مجلد المثيل كوحدة ولا يقدّم تصديرًا إلى مثيل إقلاع (launcher)، ولا قرار `R-1..R-11` يسجّل الإزالة.
- old_ref: `D:\latest_project\app\ui\main_window.py:3251` · new_ref: `src/app/pages/InstanceDetails.tsx:197`
- التوصية: إضافة إجراء تصدير إلى مثيل إقلاع، أو توثيق قرار إلغاء صريح.
- الشريحة المقترحة: M2/M3 — تصدير المثيلات.

### A-0215 — screens.md §3 #22 ProfileManagerDialog — medium — method `browser`

- الأثر: صفحة `Instances` تغطي الإنشاء والربط وفتح المجلد والمحتوى والإخفاء فقط؛ أفعال القديم `New/Edit/Rename/Duplicate/Remove/Import/Export` (وHealth Check وRepair Metadata) مفقودة، ولا أوامر Tauri ولا دوال تعديل في `core/src/db/instances.rs` تدعمها. شكل الصفحة بدل الحوار مسموح به؛ الأفعال ليست كذلك. خُفِّضت الشدة من `high` إلى `medium` في المراجعة العدائية لغياب فقدان بيانات أو كسر أمان.
- old_ref: `D:\latest_project\app\ui\main_window.py:4226` · new_ref: `src/app/pages/InstancesPage.tsx:149`
- التوصية: إضافة أوامر إعادة التسمية/النسخ/الحذف والاستيراد/التصدير مع حوار تأكيد وخطة dry-run للحذف.
- الشريحة المقترحة: M2 — صفحة المثيلات (إدارة المثيلات القائمة).

### A-0219 — screens.md §3 Saved mods (R-10) — medium — method `browser`

- الأثر: لا يوجد أي سطح للمودات المحفوظة؛ `CommandPalette` يعرض ستة وجهات فقط، وتبويب `Favourites` في المفتّش خاص باختيارات Discover لا بقائمة المثيل. مفاتيح i18n السبعة المتعلّقة كلها لـDiscover أو تصدير الحزم.
- old_ref: `D:\latest_project\app\ui\main_window.py:11059` · new_ref: `src/app/CommandPalette.tsx:17`
- التوصية: تنفيذ لوحة جانبية لقائمة المودات المحفوظة لكل مثيل (=R-10)، أو توثيق قرار `Drop`.
- الشريحة المقترحة: M2 — المودات المحفوظة (R-10).

> **حاشية المراجعة:** ملاحظات `reviewNote` توضّح سبب بقاء الشدة `medium`: المولّد يرفض `low` لوحدة
> `undocumented-deviation`، لذلك لم يخفّض المراجعُ عديدًا من المقترحات رغم اعترافه بأثر محدود
> (A-0045، A-0063، A-0072، A-0085، A-0086، A-0101، A-0102، A-0106). لم يُعتمد أي تغيير في هذه الشريحة.

## انحرافات موثّقة

هذه انحرافاتٌ عن القديم لكنها **ليست عيوبًا**: قرارها مسجَّل في `docs/spec/decision-sheet.md`،
`features.md`، `feature-parity.json`، `feature-parity.md`، `lessons.md` أو ADR ذي صلة، فلا تُحصى ضمن الـ26 finding.

- **F-125 (مخطّط الإصدار المستهدف):** `feature-parity.json` يسجّلها `partial` صراحةً: الجدول للقراءة فقط،
  ولا كتابة للهدف في سجل المثيل، ولا اختيار لإصدار مختلف، وعمود الإصدار الحالي يعرض معرّفًا بدل رقم.
  القرار `Keep`/M3، والفجوة موثّقة بالاسم؛ لذلك `A-0125` = `documented-deviation` لا finding. تبقى جزئيةً
  مُعلَنةً وليست مطابقةً كاملةً.
- **Defer (18 بندًا):** `decision-sheet.md` §B يوزّعها على M1 (F-011، F-012، F-013، F-024، F-028)،
  وM3 (F-016، F-060، F-061)، وخارج v1 (F-084، F-096، F-116، F-117، F-119، F-147، F-154، F-161)،
  وM4 (F-181). بعضها نُفِّذ فعلًا بنطاقٍ أوسع (superset، مثل F-011) فسُجّل `documented-deviation` لا عيبًا؛
  والباقي مؤجَّلٌ بقرار، لا يُعدّ خسارةً غير موثّقة.
- **Drop (14 بندًا):** `decision-sheet.md` §A يضمّ ترخيص F-175 (قرار نهائي: تطبيق مجاني بلا بصمة جهاز
  ولا نافذة تفعيل)، وواجهات زائدة (F-163، F-167، F-172)، وتصديرًا متكرّرًا (F-159، F-160)،
  وتغليف بايثون (F-176، F-177، F-178، F-180)، والوضع المحمول (F-179)، و`ownership_policy` (F-105).
  الإزالة مقصودةٌ وموثّقة، فلا تُعدّ finding.
- **M4/CurseForge:** `decision-sheet.md` §D وC-5 يقرّران إصدار v1 بمزوّد Modrinth فقط، مع CurseForge
  في M3 بمفتاح يدخله المستخدم؛ واستيراد بيانات التطبيق القديم (F-181) في M4 عبر زر اختياري بلا قراءة
  تلقائية. بُعد `packaging` (8 وحدات) كلّه `documented-deviation` وصفر findings. خطّ التغليف/التحديث
  التلقائي/التوقيع/التوزيع في `foundations.md` §7 و§10 لا يزال قرارًا مفتوحًا للإصدار M4، ولم يُبتّ فيه
  في هذه الشريحة، فلا يُصنَّف عيبًا ولا يُدعى مُنجَزًا.
- **أمثلة إعادة التصنيف بعد المراجعة العدائية:** `A-0083` (CurseForge M3 بمفتاح المستخدم)،
  `A-0128` (ADR 0007 يجعل `instance.lock.json` بديل `installed_mods.json`)،
  `A-0195` (`lessons.md` يستبدل سياق البروفايل الضمني بمعرّف صريح في كل طلب)؛ نُقلت إلى
  `documented-deviation` لأن الأساس قرارٌ موثّق، لا فقدان سلوك.

## ما لم أتحقق منه

- **وحدات `unverifiable` السبع (`performance`):** `A-0234` (B-1: بدء ساخن 2.1 s ← ≤1.0 s)،
  `A-0235` (B-2: بدء بارد 6.2 s ← ≤3.0 s)، `A-0236` (B-3: WorkingSet خامل ≤194.3 MB)،
  `A-0237` (B-4: PrivateBytes خامل ≤111.3 MB)، `A-0238` (B-5: مسح بارد لـ354 jar، 1922 ms ← ≤960 ms)،
  `A-0239` (B-6: مسح ساخن ≤540 ms)، `A-0240` (B-7: قراءة البيان ≤10 ms مقابل 4.2 ms في القديم).
  لا يوجد `bench`/`criterion` ولا أداة قياس ولا عيّنة 354 ملفًا في المستودع؛ الأدلة الموجودة على الكود
  (مثل `db::open` وذاكرة الـcache) دليل تصميمٌ لا قياسٌ زمني، وذاكرة WebView2 غير قابلة للمقارنة
  بعملية Qt. **لا تُدعى ميزانيات الأداء مُحقَّقة**، والوسم `غير مُتحقَّق` مُثبت في الدفتر.
- **قيود المتصفّح:** لقطات `.opencode/analyses/audit/shots/A-0194..A-0223.jpeg` (30 لقطة) أُخذت مع IPC
  مُقلَّد (mocked)؛ سجلّ console في هذه الوحدات يسجّل 0 أخطاء، وهذا يثبت العرض والترجمة وRTL فقط،
  ولا يثبت صحة بيانات الـ`core` ولا مسارات التكامل الحقيقية. Findings الواجهة الستة (`A-0198`،
  `A-0203`، `A-0206`، `A-0207`، `A-0215`، `A-0219`) مبنية على هذا النطاق.
- **قيود القديم:** `D:\latest_project @ 4f71f18` مصدرٌ للقراءة فقط؛ لم يُشغَّل التطبيق القديم، ولذلك
  لا يمكن إعادة إنتاج سلوكه الزمني أو التفاعلي المباشر، بل جرى التحقق من شيفرته ووثائقه.
- **F-125** لا تزال جزئيةً (انظر أعلاه)، و**M4** لا يزال قرارًا مفتوحًا؛ لم يُصلَح أي finding أو عيب
  في هذه الشريحة، بما في ذلك `D-001` (high، مفتوح).
- **بوابة `parity:check`:** فشلت في البداية لأن مخرجات التدقيق في `.opencode/analyses/audit/ws1..ws4.json`
  و`prompts/*` (مُتتبَّعة في git) كانت تُقرأ كأدلة `F-xxx` في `scripts/build-parity-matrix.mjs`، فتغيّر
  `docs/spec/parity.md` المولَّد عن الموجود. أُصلحت البوابة في `c90c743` باستبعاد `.opencode/analyses/audit/`
  و`.superpowers/` من مسح التتبّع، وتحقّقنا من `parity: up to date` عند رأس الفرع **(اجتازت)**.
- **بوابات Cargo:** طُهِّر مجلد `target/` بـ`cargo clean` (21.4 GiB) بطلب من المستخدم؛ أول بوابة cargo
  لاحقة ستُعيد البناء من الصفر.

## التوصية قبل الإطلاق

لا توجد وحدة `blocker` أو `high` بين الـ26 finding (كلها `medium`). مع ذلك:

| العنصر | الشدة | القرار |
|---|---|---|
| `D-001` — `src/app/browse/ResultsTable.tsx:40,116,193`: صنف `grid-cols-[…]` ديناميكيٌّ لا يراه Tailwind، و`display:grid` غائب، فتتراكب خلايا جدول النتائج | **high** (عيب مفتوح، ليس وحدة تدقيق) | **يُوصى بالإصلاح قبل الإصدار**؛ الإصلاح المقترح موصوف في `.opencode/analyses/audit/defects.md` |
| أي finding من الـ26 | `medium` | **يُقبل مع توثيق** للإصدار الحالي، على أن تُجدول الشرائح المقترحة أعلاه؛ لا إصلاح ضمن هذه الشريحة |
| وحدات `performance` السبع | `unverifiable` | **يُقبل مع توثيق** كقيود قياس، ولا يُدعى أي تحقّق؛ يُطلب إضافة `bench`/قياس قبل إصدار أي ادّعاء رقمي |

الخلاصة: التقرير الصالح للاعتماد هو `docs/spec/audit-findings.md` (مولَّد من الدفتر، 276 وحدة)؛
هذا التقرير الختامي يلخّصه ويضيف التوصيات. لا يُعدّ أي finding مُصلَحًا، ولا يُعدّ أي قياس أداء مُنجَزًا.
