# features.md — جدول الميزات وقرار Keep / Drop / Defer

> **المصدر:** `D:\latest_project` (تطبيق بايثون/PySide6، `master` @ `4f71f18`، 2026-10-13) — **قراءة فقط**.
> **القرار المقترح هو اقتراح الوكيل — أنت من يحسم.** ما عدا ذلك مقبول ما لم تعترض.

**عمود `القرار`:** `Keep` = نعيد التنفيذ من السلوك · `Defer` = نؤجّل · `Drop` = لا ننفّذ.

**عمود `المرحلة`** (مستويات `foundations.md` §10 — و**v1 = M1 + M2 فقط**):

| المرحلة | النطاق | عدد |
|---|---|---|
| **M1** | النواة والأمان: مزوّد Modrinth · محرك التحميل بالتحقق والكتابة الذرّية · فك آمن · SQLite · CLI | **77** |
| **M2** | الواجهة: شاشات النسخ والتصفح والتحميلات، والتقدّم عبر Channels، وكل التدفقات الأساسية | **39** |
| **M3** | التوسّع: CurseForge · المود باكات · المودات اليدوية · كشف النسخ | **44** |
| **لاحقاً** | خارج v1 | **8** |
| **—** | `Drop` | **13** |
| | **المجموع** | **181** |

**حصيلة v1 = M1 + M2 = 116 ميزة (64%).** والباقي 65 خارج v1.
**ما يحتاج قرارك صراحةً مُجمَع في [`decision-sheet.md`](./decision-sheet.md).** الباقي مقبول ما لم تعترض.

**How to read:** كل صف يحمل الدليل (ملف + دالة) حتى تتحقق بنفسك.

---

## 1. المزوّدون — providers

| ID | الميزة | السلوك المرئي للمستخدم | المجال | ملفات المصدر | القرار | المرحلة | السبب |
|---|---|---|---|---|---|---|---|
| **F-001** | بحث Modrinth مع فلاتر | بحث بالاسم + نوع المشروع + إصدار ماينكرافت + اللودر + الفئة + جهة العميل/الخادم + الترتيب + الترقيم | providers | `api/modrinth_client.py::search_projects` · `providers/base.py::search_projects` | **Keep** | M1 | جوهر المنتج |
| **F-002** | جلب مشروع/إصدار | تفاصيل + قائمة إصدارات + توافق | providers | `providers/base.py::get_project/get_version/get_project_versions` | **Keep** | M1 | أساس كل شيء |
| **F-003** | `get_best_version` | أحدث إصدار متوافق تلقائياً | providers | `modrinth_client.py::get_best_version` | **Keep** | M1 | — |
| **F-004** | بحث بالهاش (SHA-1) | يتعرّف على المودات المحلية بمقارنة SHA-1 | providers | `providers/base.py::get_versions_from_hashes` | **Keep** | M1 | يتيح فحص التحديث |
| **F-005** | بحث التحديثات دفعة واحدة | `get_latest_versions_from_hashes` بفهرسVersions | providers | `providers/base.py::get_latest_versions_from_hashes` | **Keep** | M1 | **هو** `foundations.md` §4.4 |
| **F-006** | تحميل بالهاش | نفس `(project_id, version_id)` بهاش | providers | `modrinth_client.py::get_versions_from_hashes` | **Keep** | M1 | يُستخدم في الاستيراد |
| **F-007** | معالجة حد المعدل 429 | يعرض «حاول بعد N ثانية» ولا ينام طويلاً | providers | `modrinth_client.py::_server_retry_delay` | **Keep** | M1 | سلوك مُختبَر بدقّة |
| **F-008** | **قاعدة الانتظار** | انتظار >10s ⇒ **فشل صريح** بالأوقات، **لا** نوم طويل | providers | `modrinth_client.py:130-138` | **Keep** | M1 | يمنع تجميد الواجهة |
| **F-009** | **قاعدة السنة الدوّارة** | تاريخ RFC850 بسنتين يُحلّل بـ±50 سنة | providers | `modrinth_client.py:68-78` | **Keep** | M1 | مُختبَر بحافتين |
| **F-010** | إعادة المحاولة (2 محاولات) | 429/5xx/نقل، **للطلبات القراءة فقط** | providers | `modrinth_client.py:105-126` | **Keep** | M1 | `AGENTS.md` §6 |
| **F-011** | تدهور التوافق عند 400 | بحث plugin/datapack يفشل ⇒ يعيد كـ`mod` | providers | `modrinth_client.py::search_projects:282-301` | **Defer** | M1 | إصلاح مؤقت لمصدر؛ نُعيد المحاولة مع رسالة أوضح |
| **F-012** | تحديد معدل من جهة العميل | **غير موجود** | providers | — | **Defer** | M1 | `foundations.md` §4.4 «محدّد معدل داخلي»؛ ابدأ عند بلوغ 100 طلب/دقيقة |
| **F-013** | تخزين مؤقت لنتائج البحث | **غير موجود** | providers | — | **Defer** | M1 | `foundations.md` §4.4 «تخزين مؤقت بمدة صلاحية» |
| **F-014** | **مزوّد CurseForge** | **غير موجود إطلاقاً** | providers | — (فحص `curseforge` = صفر نتيجة) | **Keep** | M3 | **قرارك: خارج v1؛ في M3 بمفتاح API يدخله المستخدم في الإعدادات.** جديد بالكامل — لا نحمل سلوكاً منه |
| **F-015** | حالة `downloadUrl` الفارغ عند CurseForge | يفتح صفحة الملف في المتصفح | providers | — | **Keep** | M3 | سلوك موصوف في `foundations.md` §4.4، **غير مُنفَّذ ولا مُختبَر** |
| **F-016** | بصمة CurseForge (MurmurHash2) | يعرّف المودات اليدوية | providers | — | **Defer** | M3 | دالة صغيرة **نكتبها** (`foundations.md` §3.2) — مرتبطة بـF-014 |
| **F-017** | رابط تنزيل فارغ | يمنع التنزيل ويُظهر سبباً | providers | `modrinth_mapping.py:46` · `batch_install_plan.py:84` | **Keep** | M1 | `project_id` وحده لا يُثبت |
| **F-018** | `User-Agent` وصفي **بمعلومات تواصل** | لا يراه المستخدم | providers | `config.py:15` | **Keep** | M1 | **قرارك: ثابت واحد في `core` قابل للضبط، قيمة مؤقتة `ModManager/0.1.0-dev (dev build)`، وبوابة تفشل بناء الإصدار النهائي ما دامت مؤقتة.** `AGENTS.md` §5.11 يفرضه |
| **F-019** | **رفض تسريب جسم الاستجابة** | لا يظهر نص خام في رسالة الخطأ | providers | `test_modrinth_api_resilience.py:42,99,107,240` | **Keep** | M1 | خصوصية |

---

## 2. التنزيلات — downloads

| ID | الميزة | السلوك المرئي | المجال | ملفات المصدر | القرار | المرحلة | السبب |
|---|---|---|---|---|---|---|---|
| **F-020** | تنزيل جماعي بتقدّم مجمَّع | شريط واحد لعدة ملفات | downloads | `services/download_manager.py` · `ui/main_window.py:13869` | **Keep** | M1 | — |
| **F-021** | ملف `.part` ثم إعادة تسمية ذرّية | لا ملف ناقص أبداً | downloads | `download_manager.py:312,375` | **Keep** | M1 | `AGENTS.md` §5.5 |
| **F-022** | التحقق SHA-1 + SHA-512 | يُهمَل الملف عند عدم التطابق | downloads | `services/artifact_verifier.py:4-5` | **Keep** | M1 | `AGENTS.md` §5.5 |
| **F-023** | فحص المساحة قبل الطلب | «لا توجد مساحة كافية» قبل البدء | downloads | `download_manager.py:68-78` | **Keep** | M1 | يمنع فشل نصف مكتوب |
| **F-024** | **عتبة المساحة دقيقة بلا هامش** | — | downloads | `download_manager.py:70-76` | **Defer** | M1 | لا هامش ولا نسبة ⇒ سهل الفشل عند الامتلاء؛ اقترح هامش 512 MiB |
| **F-025** | الاستئناف عبر `Range` | يكمل من حيث توقف | downloads | `download_manager.py:146-204` | **Keep** | M1 | — |
| **F-026** | **إعادة بداية إن لم يدعم الخادم `Range`** | بلا إلحاق تالف | downloads | `download_manager.py` (`status != 206`) | **Keep** | M1 | سلوك ممتاز |
| **F-027** | إعادة المحاولة مع `Retry-After` + backoff | صامت، ثم ينجح | downloads | `artifact_verifier.py:59-80` | **Keep** | M1 | — |
| **F-028** | **عشوائية في backoff** | — | downloads | — **غير موجودة** | **Defer** | M1 | `foundations.md` §4.5 «مع عشوائية»؛ أضف `Jitter::Full` |
| **F-029** | التحقق من كل وجهة إعادة توجيه | يمنع HTTPS⇒HTTP | downloads | `download_manager.py:206-250` | **Keep** | M1 | — |
| **F-030** | رفض استجابة HTML | «هذه صفحة خطأ» | downloads | `artifact_verifier.py:3` | **Keep** | M1 | أذكى من الهاش وحده |
| **F-031** | فرض HTTPS + قائمة بيضاء للمضيفين | — | downloads | `artifact_verifier.py::validate_download_url` | **Keep** | M1 | `AGENTS.md` §5.6 |
| **F-032** | تخطّي ملف موجود متطابق الهاش | «موجود وموثوق» بلا شبكة | downloads | `download_manager.py:295-307` | **Keep** | M1 | — |
| **F-033** | **عدم الكتابة فوق ملف مستخدم** | يُحفظ باسم `artifact-1.jar` | downloads | `download_manager.py:307` · `utils/files.py:21-32` | **Keep** | M1 | حماية إضافية |
| **F-034** | حذف `.part` عند فشل التحقق | لا يبقي ملفاً فاسداً | downloads | `download_manager.py:385` | **Keep** | M1 | — |
| **F-035** | الإيقاف المؤقت / الاستئناف / الإلغاء | `.part` محفوظ عند الإلغاء | downloads | `download_manager.py:80-135` | **Keep** | M2 | — |
| **F-036** | **الإلغاء حالة مستقلة لا نجاح** | — | downloads | **مفقود** — الإلغاء = `status="cancelled"` | **Keep** | M1 | `lessons.md` L-3؛ اجعله `IpcError::Cancelled` |
| **F-037** | أطوار معلنة قبل كل عمل | downloading→verifying→committing | downloads | `download_manager.py:324,362,371` | **Keep** | M1 | **fail-closed** في `operation_tracking.py:152` |
| **F-038** | حارس سباق الروابط الرمزية | فحص بعد الفتح وقبل الكتابة | downloads | `download_manager.py:99-104` | **Keep** | M1 | يستحق بنداً في `AGENTS.md` §5.5 |
| **F-039** | التنزيل بالتوازي | 3 افتراضياً، 1–8 | downloads | `main_window.py:1130` | **Keep** | M1 | `foundations.md` §4.5 |
| **F-040** | صفحة التنزيلات (3 تبويبات) | جارٍ / مكتمل / سجل | downloads | `ui/pages/download_manager_page.py:26` | **Keep** | M2 | كما في `foundations.md` §9 |

---

## 3. أمان الأرشيف — archive safety

| ID | الميزة | السلوك المرئي | المجال | ملفات المصدر | القرار | المرحلة | السبب |
|---|---|---|---|---|---|---|---|
| **F-041** | رفض اجتياز المسار | «مسار غير آمن» | archive | `services/safe_archive.py::safe_archive_member_name` (5) | **Keep** | M1 | `AGENTS.md` §5.6 |
| **F-042** | رفض المسار المطلق وأقراص المنصّة | — | archive |`safe_archive.py` قاعدة 4 | **Keep** | M1 | — |
| **F-043** | حدود الحجم/العدد/نسبة الضغط | «أرشيف خطير» | archive | `safe_archive.py:26-34` | **Keep** | M1 | `AGENTS.md` §5.6 |
| **F-044** | رفض الروابط الرمزية والملفات الخاصة | — | archive | `safe_archive.py::_is_link_or_special` | **Keep** | M1 | **ثغرة ويندوز معروفة** — انظر §التالي |
| **F-045** | كشف تصادمات Unicode والحالة | «عضو مكرر أو متصادم» | archive | `safe_archive.py::_member_collision_key` | **Keep** | M1 | يحمي على ويندوز |
| **F-046** | رفض الأعضاء المشفّرة | — | archive | `safe_archive.py` (`flag_bits & 0x1`) | **Keep** | M1 | — |
| **F-047** | رفض الأسماء المحجوزة في ويندوز | `CON` · `aux` · `nul` | archive | `safe_archive.py:9` | **Keep** | M1 | لاCreates ملفاً مستحيلاً |
| **F-048** | قراءة عضو بسقف | «تجاوز حد القراءة» | archive | `safe_archive.py::read_limited_member` | **Keep** | M1 | — |
| **F-049** | رفض الأرشيف الفارغ | — | archive | `artifact_verifier.py:7` | **Keep** | M1 | — |

---

## 4. المبدّل والخطة — resolver

| ID | الميزة | السلوك المرئي | المجال | ملفات المصدر | القرار | المرحلة | السبب |
|---|---|---|---|---|---|---|---|
| **F-050** | اختيار الإصدار بالإصدار+Lودر | أحدث متوافق | resolver | `services/resolver.py::resolve_project` | **Keep** | M1 | — |
| **F-051** | **لودر صارم للمودات** | **لا يسقط Fabric⇒Forge** | resolver | `resolver.py:107-110` · `test_…:412` | **Keep** | M1 | يمنع جاراً غير متوافق |
| **F-052** | تدرّج المنصّات لغير المودات | شيدرات⇒iris/optifine | resolver | `resolver.py:85-89` | **Keep** | M1 | يمنع رسائل تعارض كاذب |
| **F-053** | التبعيات المطلوبة تضاف تلقائياً | قائمة المراجعة | resolver | `resolver.py::_collect_dependency_type` | **Keep** | M1 | — |
| **F-054** | التبعيات الاختيارية **غير محددة افتراضياً** | خانات فارغة | resolver | `resolver.py` · `PHASE16_19_38` | **Keep** | M1 | — |
| **F-055** | **سلسلة المطلوبة للأختيارية** | تحديد اختيارية ⇒ تُحدَّد سلستها | resolver | `resolver.py::collect_required_dependencies_for_optional_dependencies` · `PHASE16_19_39` | **Keep** | M1 | يمنع installs ناقصة |
| **F-056** | احترام التثبيت (id/هاش/رقم) | يثبّت الإصدار المستورد | resolver | `resolver.py::_preferred_version_if_compatible` | **Keep** | M1 | جولة/رجوع |
| **F-057** | التثبيت غير المتوافق ⇒ أحدث | — | resolver | `test_…:394,425,480` | **Keep** | M1 | — |
| **F-058** | كاش لكل تشغيل | بحث أسرع | resolver | `resolver.py::ResolverRunCache` | **Keep** | M1 | **لا تعمّمه** — `lessons.md` §2 |
| **F-059** | تشخيص الكاش | «N إصابة، M missed» | resolver | `resolver.py::summary` | **Drop** | — | لا يراه المستخدم إلا مطوّراً |
| **F-060** | **كشف تعارض الإصدارات** | — | resolver | — **غير منفَّذ** | **Defer** | M3 | `foundations.md` §4.6 يطلبه؛ **القديم لا يفعله**. اقترح: تقرير لا حجب |
| **F-061** | **كشف الدورات** | — | resolver | — **غير منفَّذ** (تُقتطع بصمت) | **Defer** | M3 | نفس |
| **F-062** | حوار مراجعة التبعيات | خانات + تحذيرات | resolver | `main_window.py:1893::DownloadReviewDialog` | **Keep** | M2 | `AGENTS.md` §5.4 |
| **F-063** | **خطة التثبيت الجافة** | «سيُضاف N، M محجوب» | resolver | `services/batch_install_plan.py` | **Keep** | M1 | `AGENTS.md` §5.4 **إلزامي** |
| **F-064** | تحذيرات السلامة المحلية | 9 تحذيرات بلا حجب | resolver | `services/safety_checks.py` | **Keep** | M1 | — |
| **F-065** | تحذير الوجهة الموجودة | «سيُتخطّى أو يُسمّى» | resolver | `safety_checks.py:4` | **Keep** | M1 | — |
| **F-066** | **لا شيء يحجب في `safety_checks`** | — | resolver | — | **Keep** | M1 | **قرار واعي**: الخطة تُعرض، والحجب للواجهة |
| **F-067** | تعطيل = إعادة تسمية `.jar.disabled` | — | resolver | `foundations.md` §4.6 · `AGENTS.md` §5.12 | **Keep** | M2 | **قرارك: اسم الملف هو مصدر الحقيقة.** أي عمود `enabled` ذاكرة مشتقّة تُحسب من الاسم |

---

## 5. المود باكات — modpacks

| ID | الميزة | السلوك المرئي | المجال | ملفات المصدر | القرار | المرحلة | السبب |
|---|---|---|---|---|---|---|---|
| **F-068** | استيراد `.mrpack` | معاينة ⇒ تثبيت كامل | modpacks | `services/mrpack_service.py` · `mrpack_installer.py` | **Keep** | M3 | `foundations.md` §4.7 |
| **F-069** | **معاينة غير مُعدِّلة** | «لم يُنشأ شيء» | modpacks | `mrpack_installer.py::build_mrpack_install_plan` | **Keep** | M3 | — |
| **F-070** | تحقق صارم من الفهرس (9 فئات حواجب) | «الحزمة غير صالحة» | modpacks | `mrpack_service.py::_strict_manifest_files` | **Keep** | M3 | — |
| **F-071** | التحقق من هاش الملفات البعيدة | لا ملف غير موثوق | modpacks | `mrpack_installer.py` (`verify_artifact`) | **Keep** | M3 | — |
| **F-072** | **26 رمز خطأ مسمّى** | رسائل واضحة | modpacks | `mrpack_installer.py` | **Keep** | M3 | **نموذج ممتاز لـ`IpcError { code }`** |
| **F-073** | أولوية `overrides` < `client-overrides` | — | modpacks | `mrpack_installer.py::_register_destination` | **Keep** | M3 | — |
| **F-074** | تخطّي محتوى الخادم | — | modpacks | `mrpack_installer.py` (`server-overrides`, `env.client: unsupported`) | **Keep** | M3 | — |
| **F-075** | **نشر ذرّي بلا استبدال** | — | modpacks | `mrpack_installer.py::atomic_publish_directory_noreplace` | **Keep** | M3 | — |
| **F-076** | **تراجع على أي فشل ما قبل الالتزام** | — | modpacks | `mrpack_installer.py::rollback_mrpack_install` + إيصال مطابق | **Keep** | M3 | `test_…:1216` |
| **F-077** | التراجع **يرفض بلا إيصال مطابق** | — | modpacks | `test_rollback_refuses_missing_or_mismatched_marker` | **Keep** | M3 | يحمي ملفات المستخدم |
| **F-078** | احتياطي روابط التنزيل | ينجح من مرآة ثانية | modpacks | `mrpack_installer.py::_download_remote_item` | **Keep** | M3 | — |
| **F-079** | قائمة بيضاء لمضيفين `.mrpack` | — | modpacks | `mrpack_service.py::MRPACK_DOWNLOAD_HOSTS` | **Keep** | M3 | **بلا نطاقات فرعية** |
| **F-080** | كشف التلاعب بعد المعاينة | «تغيّرت الحزمة» | modpacks | `mrpack_installer.py` (SHA-256 للأرشيف) | **Keep** | M3 | مُختبَر **بنفس الطول** |
| **F-081** | تصدير `.mrpack` | — | modpacks | `mrpack_service.py::export_resolved_to_mrpack` | **Keep** | M3 | — |
| **F-082** | تصدير **مع** overrides | — | modpacks | — **غير منفَّذ** (تصدير فهرس فقط) | **Keep** | M3 | **قرارك:** اختيار المجلدات المضمَّنة + استبعاد ما قد يحوي أسراراً. **سلوك حقيقي مفقود** في القديم: الإعدادات المحلية تضيع |
| **F-083** | استيراد/تصدير CurseForge zip | — | modpacks | — **غير موجود** | **Keep** | M3 | `foundations.md` §4.7 · **جديد بالكامل** |
| **F-084** | packwiz | — | modpacks | — | **Defer** | later | `foundations.md` §4.7 «لاحقاً» |

---

## 6. بيانات الـjar والنسخ — jar metadata & instances

| ID | الميزة | السلوك المرئي | المجال | ملفات المصدر | القرار | المرحلة | السبب |
|---|---|---|---|---|---|---|---|
| **F-085** | قراءة `fabric.mod.json` | اسم/إصدار/Lودر من الملف المحلي | jar | `services/local_mod_metadata.py` | **Keep** | M3 | — |
| **F-086** | قراءة `META-INF/mods.toml` (Forge) | — | jar | `local_mod_metadata.py` (TOML) | **Keep** | M3 | — |
| **F-087** | قراءة `quilt.mod.json` | — | jar | `local_mod_metadata.py` | **Keep** | M3 | — |
| **F-088** | قراءة `META-INF/neoforge.mods.toml` | — | jar | `local_mod_metadata.py` | **Keep** | M3 | — |
| **F-089** | **أولوية صارمة** (أول ملف يطابق) | — | jar | `local_mod_metadata.py` | **Keep** | M3 | يمنع الالتباس |
| **F-090** | تصنيف نوع المورد | mod/shader/datapack/resourcepack/unknown | jar | `classify_local_resource_type` | **Keep** | M3 | — |
| **F-091** | **الميتافيزقا لا تُثبت هوية** | ملف مجهول **لا** يُستورد كمدار | jar | `content_inventory.py` · `test_…:1524` | **Keep** | M3 | `AGENTS.md` §5.12 |
| **F-092** | كشف الـloader من بيانات المشغّل | «اكتشاف مؤكد» | instances | `services/instance_detector.py` | **Keep** | M3 | **مصدر خادع** |
| **F-093** | **فحص الـjars بديل فقط** | يمنع التعارض الكاذب | instances | `instance_detector.py:264-266` | **Keep** | M3 | **حرفياً: `if !loaders`** |
| **F-094** | كشف تخطيط Prism/MultiMC | `.minecraft/` **و** `minecraft/` | instances | `instance_detector.py::_roots_for_selected_folder` | **Keep** | M3 | `test_prism_…` |
| **F-095** | التحقق من البروفايل (30 فحصاً) | تقرير OK/Warning/Error/Info | instances | `services/profile_validation.py` | **Keep** | M2 | **قراءة فقط** |
| **F-096** | فحص صحة النسخة | جدول مسارات قراءة فقط | instances | `main_window.py:3544::InstanceHealthDialog` | **Defer** | M2 | يتداخل مع F-095 ⇒ ادمجهما |
| **F-097** | التصنيف: مدار/خارجي/معطّل/مكرر/غير متوافق | 5 فئات | instances | `local_mod_classification.py` | **Keep** | M3 | — |
| **F-098** | **المحاور الأربعة متعامدة** | «مدار + معطّل + مكرر» معاً | instances | `content_inventory.py` · `test_…:…coexist` | **Keep** | M3 | **`struct` لا `enum` في Rust** |
| **F-099** | كشف التكرار (SHA-1 + هوية المشروع) | تحذير | instances | `content_inventory.py::_duplicate_keys` | **Keep** | M3 | — |
| **F-100** | كشف تعارض الهاش | تحذير | instances | `content_inventory.py` (`hash_mismatch`) | **Keep** | M3 | — |
| **F-101** | مسار تحديث ميتافيزقا البروفايل | — | instances | `services/profile_metadata.py` | **Keep** | M1 | `profile.json` مطبَّقة |

---

## 7. البروفايلات والنسخ الاحتياطي — profiles

| ID | الميزة | السلوك المرئي | المجال | ملفات المصدر | القرار | المرحلة | السبب |
|---|---|---|---|---|---|---|---|
| **F-102** | إنشاء بروفايل مخصّص | مجلدات + `profile.json` | profiles | `services/profile_manager.py::create_custom_profile` | **Keep** | M1 | — |
| **F-103** | ربط مجلد موجود | «لم يُنشأ أو يُنقل شيء» | profiles | `create_linked_profile` · `profile_folder_import.py` | **Keep** | M1 | — |
| **F-104** | **عمود `source`** على النسخ | تصنيف المصدر | profiles | `domain/profile_aggregate.py:9-27` | **Keep** | M1 | **قرارك:** `managed \| manual \| adopted`. **`ownership_policy` يُحذف** (انظر F-105) |
| **F-105** | ~~`ownership_policy` مُنفَّذ~~ | — | profiles | — **لا مسار تعديل يقرأه** | **Drop** | — | **قرارك:** يُدمج أثره في `source` بـ`managed \| manual \| adopted`. حقل لا يقرأه أي مسار = دَين |
| **F-106** | معاينة إنشاء على مجلد موجود | «اربط بدل الإنشاء» | profiles | `main_window.py:9642` | **Keep** | M2 | يمنع الكتابة فوق Minecraft |
| **F-107** | **تراجع إنشاء كامل** | — | profiles | `main_window.py:9767-9792` | **Keep** | M1 | — |
| **F-108** | تفعيل/تعطيل (إعادة تسمية) | معاينة ⇒ تنفيذ ⇒ تراجع | profiles | `services/profile_content_toggle.py` | **Keep** | M2 | `AGENTS.md` §5.12 |
| **F-109** | **تراجع تنفيذي عكسي** | «أُعيدت الملفات السابقة» | profiles | `profile_content_toggle.py:98` | **Keep** | M2 | `test_execute_rolls_back_prior_success…` |
| **F-110** | رفض الروابط الرمزية والتجاوز والتصادم | — | profiles | `profile_content_toggle.py:124,132,139` | **Keep** | M1 | — |
| **F-111** | حذف بمشغّل أمان **بلا حذف دائم** | «انقل إلى نسخة احتياطية» | profiles | `services/profile_content_remove.py:135` | **Keep** | M2 | **بلا بديل في Rust؟ فتح decide** |
| **F-112** | كلمة تأكيد `REMOVE` للملفات الخارجية | — | profiles | `profile_content_remove.py:191` | **Keep** | M2 | نمط ممتاز |
| **F-113** | نسخ احتياطي كامل للبروفايل | `mods/` + `config/` | profiles | `services/profile_update_backup.py` | **Keep** | M2 | copy-only |
| **F-114** | استعادة بحفظ الوجهة القائمة جانباً | — | profiles | `services/profile_backup_restore.py:227` | **Keep** | M2 | 3 طبقات حماية |
| **F-115** | **نسخ `saves` اختياري** | «محجوب افتراضياً» | profiles | `profile_update_backup.py:82-94` | **Keep** | M2 | — |
| **F-116** | مدير نسخ التحديث (لكل ملف) | — | profiles | `main_window.py:2902::BackupManagerDialog` | **Defer** | later | نظامان منفصلان؛ التوحيد أنظف |
| **F-117** | إصلاح الميتافيزقا (خطة) | «خطة ⇒ تطبيق» | profiles | `services/profile_metadata_repair.py` | **Defer** | later | قيمة متوسطية |
| **F-118** | معاينة العوالم (قراءة فقط) | جدول | profiles | `services/profile_worlds.py:76` | **Keep** | M3 | **بلا أثر جانبي** |
| **F-119** | استيراد عالم (نسخ فقط) | كتابة `IMPORT` | profiles | `profile_worlds.py:244` | **Defer** | later | نادر الاستخدام |

---
| **F-181** | استيراد بيانات التطبيق القديم (باستود) | زرّ اختياري: يقرأ نسخة `.modrinth_bulk_downloader` ويُنشئ نسخاً جديدة | profiles | — **غير موجود** | **Keep** | later | **قرارك: M4، بلا قراءة تلقائية ولا ترحيل صامت.** نسخة احتياطية قبل أي تعديل (`foundations.md` §14) |

## 8. التحديثات — updates

| ID | الميزة | السلوك المرئي | المجال | ملفات المصدر | القرار | المرحلة | السبب |
|---|---|---|---|---|---|---|---|
| **F-120** | معاينة التحديث | فرق الإصدار + التبعيات + خطة النسخ | updates | `main_window.py:7309,2668` | **Keep** | M2 | — |
| **F-121** | **إعادة تحقق قبل التنفيذ** | «تغيّر السياق ⇒ أُلغي» | updates | `main_window.py:7364` | **Keep** | M2 | عمر الخطة |
| **F-122** | عقد السلامة (5 بنود) | نص التأكيد | updates | `main_window.py:7501` | **Keep** | M2 | — |
| **F-123** | **نسخ احتياطي Yes/No/Cancel** | ثلاثي | updates | `main_window.py:7534` | **Keep** | M2 | — |
| **F-124** | تحديث بالتنزيل فقط | مجلد منفصل | updates | `main_window.py:7403` | **Keep** | M2 | لا يمسّ القائم |
| **F-125** | مخطّط الإصدار المستهدف | جدول 8 أعمدة | updates | `main_window.py:11379` | **Keep** | M3 | — |
| **F-126** | اقتراح هدف مشترك | «اقتراح» | updates | `main_window.py:11418` | **Keep** | M3 | — |
| **F-127** | **تنظيف الهدف يترك المجهول** | «ملفات مجهولة أُبقيت» | updates | `PHASE16_19_40` | **Keep** | M3 | `lessons.md` §5 |
| **F-128** | `installed_mods.json` مانيفست | — | updates | `services/installed_mods_manifest.py` | **Keep** | M2 | — |
| **F-129** | مسح المودات المحلية + استيراد | جدول 13 عموداً | updates | `main_window.py:7084,2437` | **Keep** | M3 | — |
| **F-130** | **قصّ الميتافيزقا فقط** | «أزل ما لم يعد موجوداً» | updates | `PHASE16_18_2/3` | **Keep** | M3 | ⚠ **انظر F-141** |
| **F-131** | **التمييز «موجود» عن «مؤهَّل»** | — | updates | `PHASE16_18_3` | **Keep** | M3 | `lessons.md` §4.2 — خطأ حذف بيانات |
| **F-132** | كاش لكل تشغيل في المخطّط | أسرع | updates | `PHASE16_19_17` | **Keep** | M3 | — |

---

## 9. التخزين وسجلّ العمليات — storage & operations

| ID | الميزة | السلوك المرئي | المجال | ملفات المصدر | القرار | المرحلة | السبب |
|---|---|---|---|---|---|---|---|
| **F-133** | ترحيلات SQLite المرقّمة | — | storage | `storage/migrations/__init__.py` | **Keep** | M1 | `AGENTS.md` §5.10 |
| **F-134** | **بوابة إصدار المخطط المستقبلي** | «قاعدة أحدث — لم تُعدَّل» | storage | `migration_runner.py:245-279` | **Keep** | M1 | **`foundations.md` §14 يطلبه صراحةً** · مُختبَر بايت-ببايت |
| **F-135** | نسخة احتياطية قبل الترحيل | `.bak` بجانب القاعدة | storage | `migration_runner.py::_create_backup` | **Keep** | M1 | `AGENTS.md` §5.10 |
| **F-136** | **نشر النسخة بلا استبدال** | — | storage | `_publish_backup` (`MoveFileExW` / `os.link`) | **Keep** | M1 | — |
| **F-137** | **fail-closed عند القفل** | «مشغول» بلا كشف `database is locked` | storage | `migration_runner.py` | **Keep** | M1 | — |
| **F-138** | **تنقية رسائل الترحيل** | اسم ملف لا مسار | storage | `_safe_error_message` | **Keep** | M1 | خصوصية |
| **F-139** | **فشل قبل بناء النافذة** | «فشل قاعدة البيانات» + خروج 2 | storage | `main_window.py:14241-14258` | **Keep** | M1 | `AGENTS.md` §5.2 |
| **F-140** | **WAL** | — | storage | — **غير مضبوط** | **Keep** | M1 | **`AGENTS.md` §5.10 يفرضه؛ القديم يخالفه** |
| **F-141** | **سجلّ عمليات ذرّي (JSON)** | — | operations | `services/operation_journal.py` | **Keep** | M1 | — |
| **F-142** | انتقالات مُقيّدة للعمليات والعناصر | — | operations | `operation_models.py:53-94` | **Keep** | M1 | مُختبَر (5×5 و8×8) |
| **F-143** | **كشف مجلد عمليات غير منتهية** | «عملية مقاطعة» | operations | `main_window.py:4588` | **Keep** | M1 | basename فقط |
| **F-144** | **القفل عبر العمليات** | «مشغول» | operations | `services/operation_lease.py` | **Keep** | M2 | يمنع تضارب نسختين |
| **F-145** | مركز العمليات (قراءة فقط) | جدول تنبيهات | operations | `ui/operation_center.py:179` | **Keep** | M2 | — |
| **F-146** | **تقييم الأدلة — أعداد فقط** | «لا يمكن تحديده» بدل التخمين | operations | `operation_recovery_assessment.py` | **Keep** | M3 | `recovery_action_allowed` **دائماً false** |
| **F-147** | **تعليق كـ«مُهجر»** | — | operations | `PHASE16_8` | **Defer** | later |Procedure معقّد؛ قيمة متوسطية |
| **F-148** | تنقية الأسرار من السجلات | — | logging | `services/log_sanitizer.py` | **Keep** | M1 | `foundations.md` §14 |
| **F-149** | **السجل المحلي كامل وغير منقّى** | تشخيص كامل محلياً | logging | `main_window.py::log` | **Keep** | M1 | **حدّ مقصود وموثّق** |
| **F-150** | **فشل مغلق بلا مخرج خام** | — | logging | `test_sanitizer_failure_aborts_report…` | **Keep** | M1 | — |
| **F-151** | إعدادات محفوظة (21 مفتاحاً) | — | settings | `storage/repositories.py:595` | **Keep** | M2 | اقتراح **`foundations.md` §5: ملف JSON** |
| **F-152** | سجلّ التنزيلات | جدول + بحث | settings | `services/download_manager_service.py` | **Keep** | M2 | — |
| **F-153** | **فهرس على سجلّ التنزيلات** | — | settings | — **غير موجود** (4 × `LIKE '%…%'`) | **Keep** | M2 | مسح كامل مضمون |
| **F-154** | المفضّلة | نجمة + قائمة | settings | `storage/repositories.py:221` | **Defer** | later | **F-104/129 يغطّيانjid**؛ بطاقة منفصلة |

---

## 10. الاستيراد والتصدير — import/export

| ID | الميزة | السلوك المرئي | المجال | ملفات المصدر | القرار | المرحلة | السبب |
|---|---|---|---|---|---|---|---|
| **F-155** | تصدير بروفايل zip | معاينة ⇒ كتابة | import/export | `services/profile_export.py:240,329` | **Keep** | M2 | — |
| **F-156** | **قائمة الاستبعاد** | لا saves ولا أسرار ولا روابط | import/export | `profile_export.py:34-87` | **Keep** | M2 | **انقلها حرفياً** |
| **F-157** | `export_manifest.json` | سياسة مقروءة آلياً | import/export | `profile_export.py:613` | **Keep** | M2 | ممتاز |
| **F-158** | **استيراد ميتا فقط بلا استخراج** | «لم يُستخرج شيء» | import/export | `profile_export.py:154` | **Keep** | M2 | `AGENTS.md` §5.6 |
| **F-159** | تصدير قائمة المودات (MD/CSV/TXT) | — | import/export | `services/profile_mod_export.py` | **Drop** | — | لا يُكتسب بالبحث؛ زر موجود في **F-020** |
| **F-160** | استيراد/تصدير JSON | — | import/export | `main_window.py:12632,12687` | **Drop** | — | superseded by zip |
| **F-161** | اختيار هدف عالم لقاعدة البيانات | — | import/export | `main_window.py:1387::DataPackWorldTargetDialog` | **Defer** | later | niche |

---

## 11. الواجهة والتنقّل — ui

| ID | الميزة | السلوك المرئي | المجال | ملفات المصدر | القرار | المرحلة | السبب |
|---|---|---|---|---|---|---|---|
| **F-162** | شريط تنقّل + صفحات مكدّسة | 5 صفحات | ui | `main_window.py:5301,5287` | **Keep** | M2 | Wouter + React |
| **F-163** | لوحة **Home** | آخر 3 بروفايلات | ui | `ui/pages/home_page.py:239` | **Drop** | — | قائمة البروفايلات هي **F-162** |
| **F-164** | صفحة Discover | بحث + نتائج + مفتش | ui | `main_window.py:4881-5269` | **Keep** | M2 | **الشاشة 3** في `foundations.md` §9 |
| **F-165** | جدول نتائج (13 عموداً) | فرز/تحديد بلا إعادة تحميل | ui | `ui/pages/discover_results.py:45` | **Keep** | M2 | TanStack Virtual |
| **F-166** | ترقيم «Load more» | تمرير لا نهائي بـ120ms | ui | `test_discover_incremental_loading.py` | **Keep** | M2 | TanStack Query |
| **F-167** | **سجلّ تنقّل 32 مدخلاً** | Alt+← / Alt+→ | ui | `application/navigation_controller.py:7` | **Drop** | — | Wouter يستبدله؛ بلا قيمة |
| **F-168** | **مراقب نظام الملفات (تلميح)** | «المحتوى قد يكون قديماً» | ui | `ui/profile_content_watcher.py` | **Keep** | M2 | يمنع الالتباس؛ **بلا مسح تلقائي** |
| **F-169** | لوحة تنقّل لوحة المفاتيح (cmdk) | — | ui | — **غير موجود** | **Keep** | M2 | في `AGENTS.md` §4 |
| **F-170** | الثيم واللون | فاتح/داكن/النظام | ui | `main_window.py:1026` | **Keep** | M2 | — |
| **F-171** | **اختصارات لوحة المفاتيح** | — | ui | جزئي فقط (Alt+←/→) | **Keep** | M2 | `foundations.md` §9 |
| **F-172** | شريط عنوان مخصّص (إطار مخفي) | — | ui | `ui/title_bar.py:26` | **Drop** | — | Tauri يعطيه مجاناً؛ 30 اختباراً سقطت |
| **F-173** | RTL + i18n | — | ui | — **غير موجود إطلاقاً** | **Keep** | M2 | **`AGENTS.md` §5.9 يفرضه من أول شاشة** |
| **F-174** | تخطيط متجاوب | — | ui | — **غير موجود** | **Keep** | M2 | تفرضه المنصّتان |

---

## 12. أشياء كبيرة مرشّحة للحذف — drop

| ID | الميزة | الحجم | ملفات المصدر | القرار | المرحلة | السبب |
|---|---|---|---|---|---|---|
| **F-175** | **الترخيص والتفعيل دون اتصال** | `app/licensing.py` 350 سطراً · **16 مرحلة** · 14 وثيقة | `licensing.py` · `main_window.py:14141` · `packaging/license/*` | **Drop** | — | **قرارك: Drop نهائي.** تطبيق مجاني — لا نظام ترخيص ولا بصمة جهاز ولا نافذة تفعيل |
| **F-176** | **بناء PyInstaller + Inno + AppImage** | `build.ps1` · 20+ مرحلة | `packaging/**` | **Drop** | — | Tauri bundler يحلّها (`foundations.md` §7) |
| **F-177** | **نماذج العامة/الخاصة** | 3 مرحلة | `PHASE16_5/6/7` | **Drop** | — | متعلّق بـF-175 |
| **F-178** | **تشخيص ذاتي + قوالب بناء** | `diagnostics.py` 553 · 4 مرحلة | `diagnostics.py` | **Drop** | — | يستحق فقط مع الترخيص |
| **F-179** | **وضع محمول** | `portable.flag` | `config.py:27` | **Drop** | — | `directories` يكفي |
| **F-180** | **CI لاختبارات Python** | — | `.github/workflows/` | **Drop** | — | لا CI أصلاً؛ يكتب سطر واحد لـCargo |

---

## 13. ملخّص الأرقام

> **تصحيحان تراكميّان:** كتبتُ أولاً «~132 / ~20 / ~14» (تقديرات خاطئة)، ثم صحّحتُها إلى «151 / 19 / 13». **بعد قرارك بإلغاء الترخيص نهائياً (F-175 ⇒ Drop) وتضافة F-181، العدد النهائي: 152 / 16 / 13** من 181 صفاً.

| القرار | العدد | M1 | M2 | M3 | لاحقاً |
|---|---|---|---|---|---|
| **Keep** | **152** | 72 | 38 | 41 | 0 |
| **Defer** | **16** | 0 | 0 | 4 | 8 |
| **Drop** | **13** | — | — | — | — |
| **المجموع** | **181** | 77 | 39 | 44 | 8 |

**أكبر Drop قائمة:** الترخيص + التغليف + التنقّل الخلفي + شريط العنوان المخصّص + المفضّلة + تصدير JSON ⇒ **~3,500 سطر بايثون + 20 مرحلة توثيق**.

**أكبر Defer قائمة:** CurseForge جزئياً · كشف التعارض/الدورات · overrides في التصدير · الوضع المحمول · مدير نسخ التحديث · إصلاح الميتافيزقا.

---

## 14. ⚠️ غير مُتحقَّق

| البند |
|---|
| **لم أُشغّل أي اختبار** — كل الأرقام من قراءة المصدر |
| `app\ui\workers.py` (3,035 سطراً) **لم يُقرأ** — أجسام العمال مجهولة |
| `main_window.py` (13,061) **مستخرَج بالبنية فقط** |
| **CurseForge كاملاً (F-014/015/016/083) بلا أي سطر مرجعي** — قرارات تصميمية لا نقل |
| `docs\modrinth-app-gap-analysis.md` (97 KB) لم يُقرأ |
| **F-130 «قصّ الميتافيزقا» — خطر حذف بيانات.** في Rust: **Dry-run إلزامي** (`AGENTS.md` §5.4) |
| **F-059/F-163/F-167 محذوفات رأي** — قد تكون مطلوبة سطرياً |
