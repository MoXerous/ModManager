# مصفوفة قابلية التتبّع للمواصفات — ModManager

> **مولَّدة آليًّا**: `node scripts/build-parity-matrix.mjs` — تقرأ `docs/spec/features.md`
> وتمسح الشجرة عن حضور كل معرّف `F-xxx`. بلا اعتماديات خارجية.

## ما الذي تقيسه هذه المصفوفة

قياسُ **قابلية التتبّع**: هل يحمل أثرُ السلوك معرّفَ مواصفته في مكانٍ خاضعٍ للبحث؟
هذا عُرفُ المستودع نفسه (`AGENTS.md` §6: «أسماءُ الاختبارات تحمل معرّف المواصفة»).

> ⚠ **ليس قياسَ تنفيذ**: غيابُ المعرّف من الكود **لا يعني** غيابَ السلوك؛ يعني أن الدليل
> عليه غيرُ مُسمّى. حالةُ التنفيذ موثَّقة في `docs/todo.md`.

## الملخّص

- **181** بندًا في المواصفات · **152** منها `Keep` · 16 `Defer` · 7 `Drop`.

| الأثر | بنود `Keep` | بنود كلُّها |
|---|---|---|
| مُختبَر بمعرّف المواصفة | 81 | 81 |
| مُنفَّذ ومُشار إليه في الكود | 9 | 10 |
| مذكور في الاختبارات (بلا معرّف في الاسم) | 0 | 0 |
| إشارة في الوثائق فقط | 2 | 3 |
| بلا أثر | 60 | 64 |

**81 من 152** بندًا `Keep` له اختبارٌ يحمل معرّفه · **9** مذكورٌ في كود الإنتاج · **62** بلا أثرٍ قابلٍ للتتبّع.

## حسب المرحلة

| المرحلة | بنود Keep | مُختبَر بمعرّف | مُشار إليه في الكود | بلا أثر |
|---|---|---|---|---|
| M1 | 72 | 55 | 3 | 14 |
| M2 | 38 | 11 | 2 | 25 |
| M3 | 41 | 14 | 4 | 23 |
| later | 1 | 1 | 0 | 0 |

## بنود `Keep` بلا أثر قابلٍ للتتبّع — قائمة hygiene

> هذه ليست قائمةَ «غير مُنفَّذ». إنها قائمةُ **أدلّةٍ بلا أسماء** — إما اختبارٌ يحمل معرّفًا غير مطابق، أو تنفيذٌ بلا إشارة.

| المعرّف | البند | المجال | المرحلة |
|---|---|---|---|
| **F-003** | `get_best_version` | providers | M1 |
| **F-006** | تحميل بالهاش | providers | M1 |
| **F-009** | **قاعدة السنة الدوّارة** | providers | M1 |
| **F-015** | حالة `downloadUrl: null` عند CurseForge | providers | M3 |
| **F-020** | تنزيل جماعي بتقدّم مجمَّع | downloads | M1 |
| **F-022** | التحقق SHA-1 + SHA-512 | downloads | M1 |
| **F-025** | الاستئناف عبر `Range` | downloads | M1 |
| **F-029** | التحقق من كل وجهة إعادة توجيه | downloads | M1 |
| **F-036** | **الإلغاء حالة مستقلة لا نجاح** | downloads | M1 |
| **F-037** | أطوار معلنة قبل كل عمل | downloads | M1 |
| **F-038** | حارس سباق الروابط الرمزية | downloads | M1 |
| **F-040** | صفحة التنزيلات (3 تبويبات) | downloads | M2 |
| **F-068** | استيراد `.mrpack` | modpacks | M3 |
| **F-069** | **معاينة غير مُعدِّلة** | modpacks | M3 |
| **F-070** | تحقق صارم من الفهرس (9 فئات حواجب) | modpacks | M3 |
| **F-071** | التحقق من هاش الملفات البعيدة | modpacks | M3 |
| **F-072** | **26 رمز خطأ مسمّى** | modpacks | M3 |
| **F-075** | **نشر ذرّي بلا استبدال** | modpacks | M3 |
| **F-077** | التراجع **يرفض بلا إيصال مطابق** | modpacks | M3 |
| **F-078** | احتياطي روابط التنزيل | modpacks | M3 |
| **F-079** | قائمة بيضاء لمضيفين `.mrpack` | modpacks | M3 |
| **F-087** | قراءة `quilt.mod.json` | jar | M3 |
| **F-094** | كشف تخطيط Prism/MultiMC | instances | M3 |
| **F-095** | التحقق من البروفايل (30 فحصاً) | instances | M2 |
| **F-097** | التصنيف: مدار/خارجي/معطّل/مكرر/غير متوافق | instances | M3 |
| **F-098** | **المحاور الأربعة متعامدة** | instances | M3 |
| **F-099** | كشف التكرار (SHA-1 + هوية المشروع) | instances | M3 |
| **F-100** | كشف تعارض الهاش | instances | M3 |
| **F-108** | تفعيل/تعطيل (إعادة تسمية) | profiles | M2 |
| **F-109** | **تراجع تنفيذي عكسي** | profiles | M2 |
| **F-113** | نسخ احتياطي كامل للبروفايل | profiles | M2 |
| **F-114** | استعادة بحفظ الوجهة القائمة جانباً | profiles | M2 |
| **F-115** | **نسخ `saves` اختياري** | profiles | M2 |
| **F-118** | معاينة العوالم (قراءة فقط) | profiles | M3 |
| **F-121** | **إعادة تحقق قبل التنفيذ** | updates | M2 |
| **F-122** | عقد السلامة (5 بنود) | updates | M2 |
| **F-123** | **نسخ احتياطي Yes/No/Cancel** | updates | M2 |
| **F-124** | تحديث بالتنزيل فقط | updates | M2 |
| **F-125** | مخطّط الإصدار المستهدف | updates | M3 |
| **F-126** | اقتراح هدف مشترك | updates | M3 |
| **F-127** | **تنظيف الهدف يترك المجهول** | updates | M3 |
| **F-130** | **قصّ الميتافيزقا فقط** | updates | M3 |
| **F-131** | **التمييز «موجود» عن «مؤهَّل»** | updates | M3 |
| **F-132** | كاش لكل تشغيل في المخطّط | updates | M3 |
| **F-137** | **fail-closed عند القفل** | storage | M1 |
| **F-138** | **تنقية رسائل الترحيل** | storage | M1 |
| **F-139** | **فشل قبل بناء النافذة** | storage | M1 |
| **F-144** | **القفل عبر العمليات** | operations | M2 |
| **F-145** | مركز العمليات (قراءة فقط) | operations | M2 |
| **F-150** | **فشل مغلق بلا مخرج خام** | logging | M1 |
| **F-155** | تصدير بروفايل zip | import/export | M2 |
| **F-157** | `export_manifest.json` | import/export | M2 |
| **F-158** | **استيراد ميتا فقط بلا استخراج** | import/export | M2 |
| **F-162** | شريط تنقّل + صفحات مكدّسة | ui | M2 |
| **F-164** | صفحة Discover | ui | M2 |
| **F-165** | جدول نتائج (13 عموداً) | ui | M2 |
| **F-166** | ترقيم «Load more» | ui | M2 |
| **F-168** | **مراقب نظام الملفات (تلميح)** | ui | M2 |
| **F-169** | لوحة تنقّل لوحة المفاتيح (cmdk) | ui | M2 |
| **F-170** | الثيم واللون | ui | M2 |
| **F-173** | RTL + i18n | ui | M2 |
| **F-174** | تخطيط متجاوب | ui | M2 |

## التفاصيل

### providers

| المعرّف | البند | القرار | المرحلة | الأثر | الدليل |
|---|---|---|---|---|---|
| **F-001** | بحث Modrinth مع فلاتر | Keep | M1 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/providers/modrinth.rs |
| **F-002** | جلب مشروع/إصدار | Keep | M1 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/providers/modrinth.rs |
| **F-003** | `get_best_version` | Keep | M1 | بلا أثر | — |
| **F-004** | بحث بالهاش (SHA-1) | Keep | M1 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/providers/modrinth.rs |
| **F-005** | بحث التحديثات دفعة واحدة | Keep | M1 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/providers/modrinth.rs، src-tauri/src/progress.rs · كود: core/src/providers/mod.rs |
| **F-006** | تحميل بالهاش | Keep | M1 | بلا أثر | — |
| **F-007** | معالجة حد المعدل 429 | Keep | M1 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/providers/modrinth.rs |
| **F-008** | **قاعدة الانتظار** | Keep | M1 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/providers/modrinth.rs |
| **F-009** | **قاعدة السنة الدوّارة** | Keep | M1 | بلا أثر | — |
| **F-010** | إعادة المحاولة (2 محاولات) | Keep | M1 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/providers/modrinth.rs |
| **F-011** | تدهور التوافق عند 400 | Defer | M1 | مؤجَّل | اختبار بمعرّف: core/src/providers/modrinth.rs |
| **F-012** | تحديد معدل من جهة العميل | Defer | M1 | مؤجَّل | اختبار (ذكر): AGENTS.md · وثائق: AGENTS.md |
| **F-013** | تخزين مؤقت لنتائج البحث | Defer | M1 | مؤجَّل | — |
| **F-014** | **مزوّد CurseForge** | Keep | M3 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/providers/curseforge/mod.rs، core/src/providers/curseforge/secret.rs (+3) · كود: src-tauri/src/dto.rs، src-tauri/src/providers/tests.rs |
| **F-015** | حالة `downloadUrl: null` عند CurseForge | Keep | M3 | إشارة في الوثائق فقط | وثائق: docs/adr/0004-curseforge-provider.md |
| **F-016** | بصمة CurseForge (`fileFingerprint`) | Defer | M3 | مؤجَّل | وثائق: docs/adr/0004-curseforge-provider.md |
| **F-017** | رابط تنزيل فارغ | Keep | M1 | مُنفَّذ ومُشار إليه في الكود | كود: core/src/providers/curseforge/mod.rs، core/src/providers/mod.rs (+1) · وثائق: docs/adr/0004-curseforge-provider.md |
| **F-018** | `User-Agent` وصفي **بمعلومات تواصل** | Keep | M1 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/lib.rs · كود: core/src/lib.rs |
| **F-019** | **رفض تسريب جسم الاستجابة** | Keep | M1 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/providers/modrinth.rs، src-tauri/src/commands/tests.rs · كود: core/src/db/download_history.rs، core/src/resolver/resolve/mod.rs (+1) |
### downloads

| المعرّف | البند | القرار | المرحلة | الأثر | الدليل |
|---|---|---|---|---|---|
| **F-020** | تنزيل جماعي بتقدّم مجمَّع | Keep | M1 | بلا أثر | — |
| **F-021** | ملف `.part` ثم إعادة تسمية ذرّية | Keep | M1 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/downloader/client.rs، core/src/safety/atomic.rs · كود: core/src/safety/atomic.rs |
| **F-022** | التحقق SHA-1 + SHA-512 | Keep | M1 | بلا أثر | — |
| **F-023** | فحص المساحة قبل الطلب | Keep | M1 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/downloader/client.rs |
| **F-024** | **عتبة المساحة دقيقة بلا هامش** | Defer | M1 | مؤجَّل | — |
| **F-025** | الاستئناف عبر `Range` | Keep | M1 | بلا أثر | — |
| **F-026** | **إعادة بداية إن لم يدعم الخادم `Range`** | Keep | M1 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/downloader/client.rs · كود: core/src/downloader/client.rs |
| **F-027** | إعادة المحاولة مع `Retry-After` + backoff | Keep | M1 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/downloader/client.rs، core/src/downloader/retry.rs |
| **F-028** | **عشوائية في backoff** | Defer | M1 | مؤجَّل | — |
| **F-029** | التحقق من كل وجهة إعادة توجيه | Keep | M1 | بلا أثر | — |
| **F-030** | رفض استجابة HTML | Keep | M1 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/downloader/client.rs |
| **F-031** | فرض HTTPS + قائمة بيضاء للمضيفين | Keep | M1 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: cli/src/commands/mod.rs، core/src/downloader/client.rs |
| **F-032** | تخطّي ملف موجود متطابق الهاش | Keep | M1 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/downloader/client.rs · كود: core/src/downloader/client.rs، core/src/modpack/exec.rs |
| **F-033** | **عدم الكتابة فوق ملف مستخدم** | Keep | M1 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/downloader/client.rs · كود: core/src/downloader/client.rs |
| **F-034** | حذف `.part` عند فشل التحقق | Keep | M1 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/downloader/client.rs · كود: core/src/downloader/client.rs، core/src/safety/atomic.rs · وثائق: docs/plans/2026-10-15-master-plan.md |
| **F-035** | الإيقاف المؤقت / الاستئناف / الإلغاء | Keep | M2 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/downloader/client.rs · كود: core/src/downloader/client.rs · وثائق: docs/plans/2026-10-15-master-plan.md |
| **F-036** | **الإلغاء حالة مستقلة لا نجاح** | Keep | M1 | بلا أثر | — |
| **F-037** | أطوار معلنة قبل كل عمل | Keep | M1 | بلا أثر | — |
| **F-038** | حارس سباق الروابط الرمزية | Keep | M1 | بلا أثر | — |
| **F-039** | التنزيل بالتوازي | Keep | M1 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/downloader/client.rs · كود: core/src/downloader/client.rs، core/src/downloader/mod.rs (+1) |
| **F-040** | صفحة التنزيلات (3 تبويبات) | Keep | M2 | بلا أثر | — |
### archive

| المعرّف | البند | القرار | المرحلة | الأثر | الدليل |
|---|---|---|---|---|---|
| **F-041** | رفض اجتياز المسار | Keep | M1 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/safety/extract.rs، core/src/safety/path.rs · كود: core/src/safety/extract.rs، core/src/safety/path.rs |
| **F-042** | رفض المسار المطلق وأقراص المنصّة | Keep | M1 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/downloader/client.rs، core/src/safety/extract.rs (+1) · كود: core/src/downloader/client.rs، core/src/safety/inspect.rs |
| **F-043** | حدود الحجم/العدد/نسبة الضغط | Keep | M1 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/safety/extract.rs، core/src/safety/inspect.rs · كود: core/src/safety/inspect.rs، core/src/safety/path.rs |
| **F-044** | رفض الروابط الرمزية والملفات الخاصة | Keep | M1 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/safety/extract.rs، core/src/safety/inspect.rs · كود: core/src/safety/extract.rs، core/src/safety/inspect.rs |
| **F-045** | كشف تصادمات Unicode والحالة | Keep | M1 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/safety/inspect.rs |
| **F-046** | رفض الأعضاء المشفّرة | Keep | M1 | مُنفَّذ ومُشار إليه في الكود | كود: core/src/safety/inspect.rs |
| **F-047** | رفض الأسماء المحجوزة في ويندوز | Keep | M1 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/safety/extract.rs، core/src/safety/path.rs · كود: core/src/safety/extract.rs، core/src/safety/path.rs |
| **F-048** | قراءة عضو بسقف | Keep | M1 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/safety/inspect.rs · كود: core/src/safety/inspect.rs |
| **F-049** | رفض الأرشيف الفارغ | Keep | M1 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/safety/extract.rs · كود: core/src/safety/extract.rs |
### resolver

| المعرّف | البند | القرار | المرحلة | الأثر | الدليل |
|---|---|---|---|---|---|
| **F-050** | اختيار الإصدار بالإصدار+Lودر | Keep | M1 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/resolver/graph/tests.rs، core/src/resolver/resolve/tests.rs |
| **F-051** | **لودر صارم للمودات** | Keep | M1 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/resolver/graph/tests.rs، core/src/resolver/resolve/tests.rs · كود: core/src/resolver/graph/mod.rs |
| **F-052** | تدرّج المنصّات لغير المودات | Keep | M1 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/resolver/graph/tests.rs |
| **F-053** | التبعيات المطلوبة تضاف تلقائياً | Keep | M1 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/resolver/graph/tests.rs، core/src/resolver/resolve/tests.rs |
| **F-054** | التبعيات الاختيارية **غير محددة افتراضياً** | Keep | M1 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/resolver/graph/tests.rs · كود: core/src/resolver/graph/mod.rs |
| **F-055** | **سلسلة المطلوبة للأختيارية** | Keep | M1 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/resolver/graph/tests.rs · كود: core/src/resolver/graph/mod.rs |
| **F-056** | احترام التثبيت (id/هاش/رقم) | Keep | M1 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/resolver/graph/tests.rs، core/src/resolver/resolve/tests.rs · كود: core/src/resolver/graph/mod.rs |
| **F-057** | التثبيت غير المتوافق ⇒ أحدث | Keep | M1 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/resolver/graph/tests.rs، core/src/resolver/resolve/tests.rs · كود: core/src/resolver/graph/mod.rs |
| **F-058** | كاش لكل تشغيل | Keep | M1 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/resolver/resolve/tests.rs · كود: core/src/resolver/cache.rs |
| **F-059** | تشخيص الكاش | Drop | — | متروك | كود: core/src/resolver/cache.rs |
| **F-060** | **كشف تعارض الإصدارات** | Defer | M3 | مؤجَّل | كود: core/src/resolver/mod.rs |
| **F-061** | **كشف الدورات** | Defer | M3 | مؤجَّل | كود: core/src/resolver/batch.rs، core/src/resolver/graph/mod.rs (+1) |
| **F-062** | حوار مراجعة التبعيات | Keep | M2 | مُنفَّذ ومُشار إليه في الكود | كود: src/app/pages/InstallPlanDialog.tsx |
| **F-063** | **خطة التثبيت الجافة** | Keep | M1 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/downloader/batch_tests.rs، core/src/resolver/plan/tests.rs · كود: core/src/downloader/batch.rs، core/src/resolver/plan/mod.rs (+6) · اختبار (ذكر): core/src/downloader/batch_tests.rs |
| **F-064** | تحذيرات السلامة المحلية | Keep | M1 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/resolver/plan/tests.rs |
| **F-065** | تحذير الوجهة الموجودة | Keep | M1 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/resolver/plan/tests.rs، src-tauri/src/commands/tests.rs · كود: core/src/resolver/batch.rs، core/src/resolver/plan/mod.rs (+1) |
| **F-066** | **لا شيء يحجب في `safety_checks`** | Keep | M1 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/resolver/plan/tests.rs · كود: core/src/resolver/plan/mod.rs، src-tauri/src/dto.rs (+1) |
| **F-067** | تعطيل = إعادة تسمية `.jar.disabled` | Keep | M2 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/instances/tests.rs، core/src/instances/toggle/tests.rs (+1) · كود: core/src/instances/mod.rs، core/src/instances/toggle/mod.rs |
### modpacks

| المعرّف | البند | القرار | المرحلة | الأثر | الدليل |
|---|---|---|---|---|---|
| **F-068** | استيراد `.mrpack` | Keep | M3 | إشارة في الوثائق فقط | وثائق: docs/adr/0005-pack-url-policy.md |
| **F-069** | **معاينة غير مُعدِّلة** | Keep | M3 | بلا أثر | — |
| **F-070** | تحقق صارم من الفهرس (9 فئات حواجب) | Keep | M3 | بلا أثر | — |
| **F-071** | التحقق من هاش الملفات البعيدة | Keep | M3 | بلا أثر | — |
| **F-072** | **26 رمز خطأ مسمّى** | Keep | M3 | بلا أثر | — |
| **F-073** | أولوية `overrides` < `client-overrides` | Keep | M3 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/modpack/archive_tests.rs · كود: core/src/modpack/archive.rs، core/src/modpack/mrpack.rs |
| **F-074** | تخطّي محتوى الخادم | Keep | M3 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/modpack/archive_tests.rs · كود: core/src/modpack/archive.rs، core/src/modpack/mrpack.rs (+3) |
| **F-075** | **نشر ذرّي بلا استبدال** | Keep | M3 | بلا أثر | — |
| **F-076** | **تراجع على أي فشل ما قبل الالتزام** | Keep | M3 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/journal/store/tests.rs، core/src/journal/tests.rs (+4) · كود: core/src/journal/mod.rs، core/src/journal/store/mod.rs (+8) · اختبار (ذكر): core/src/modpack/exec_tests.rs، core/src/modpack/recovery_tests.rs (+1) |
| **F-077** | التراجع **يرفض بلا إيصال مطابق** | Keep | M3 | بلا أثر | — |
| **F-078** | احتياطي روابط التنزيل | Keep | M3 | بلا أثر | — |
| **F-079** | قائمة بيضاء لمضيفين `.mrpack` | Keep | M3 | بلا أثر | — |
| **F-080** | كشف التلاعب بعد المعاينة | Keep | M3 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/modpack/recovery_tests.rs · كود: src-tauri/src/commands/pack.rs · اختبار (ذكر): core/src/modpack/recovery_tests.rs |
| **F-081** | تصدير `.mrpack` | Keep | M3 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/modpack/export_tests.rs، src-tauri/src/commands/export_tests.rs · كود: core/src/modpack/export.rs، src-tauri/src/commands/pack.rs (+3) · اختبار (ذكر): core/src/modpack/export_tests.rs |
| **F-082** | تصدير **مع** overrides | Keep | M3 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/modpack/export_tests.rs، src-tauri/src/commands/export_tests.rs · كود: core/src/modpack/export.rs، src-tauri/src/commands/pack.rs (+5) · اختبار (ذكر): core/src/modpack/export_tests.rs · وثائق: docs/adr/0003-scope-decisions.md |
| **F-083** | استيراد/تصدير CurseForge zip | Keep | M3 | مُنفَّذ ومُشار إليه في الكود | كود: core/src/modpack/export.rs · وثائق: docs/adr/0005-pack-url-policy.md |
| **F-084** | packwiz | Defer | later | مؤجَّل | — |
### jar

| المعرّف | البند | القرار | المرحلة | الأثر | الدليل |
|---|---|---|---|---|---|
| **F-085** | قراءة `fabric.mod.json` | Keep | M3 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/jar_reader/tests.rs |
| **F-086** | قراءة `META-INF/mods.toml` (Forge) | Keep | M3 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/jar_reader/tests.rs |
| **F-087** | قراءة `quilt.mod.json` | Keep | M3 | بلا أثر | — |
| **F-088** | قراءة `META-INF/neoforge.mods.toml` | Keep | M3 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/jar_reader/tests.rs |
| **F-089** | **أولوية صارمة** (أول ملف يطابق) | Keep | M3 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/jar_reader/tests.rs |
| **F-090** | تصنيف نوع المورد | Keep | M3 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/jar_reader/tests.rs · كود: core/src/instances/mod.rs |
| **F-091** | **الميتافيزقا لا تُثبت هوية** | Keep | M3 | مُنفَّذ ومُشار إليه في الكود | كود: core/src/jar_reader/mod.rs |
### instances

| المعرّف | البند | القرار | المرحلة | الأثر | الدليل |
|---|---|---|---|---|---|
| **F-092** | كشف الـloader من بيانات المشغّل | Keep | M3 | مُنفَّذ ومُشار إليه في الكود | كود: core/src/instances/registry.rs، src-tauri/src/commands.rs (+2) |
| **F-093** | **فحص الـjars بديل فقط** | Keep | M3 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/jar_reader/tests.rs · كود: src-tauri/src/commands.rs، src-tauri/src/dto.rs (+1) |
| **F-094** | كشف تخطيط Prism/MultiMC | Keep | M3 | بلا أثر | — |
| **F-095** | التحقق من البروفايل (30 فحصاً) | Keep | M2 | بلا أثر | — |
| **F-096** | فحص صحة النسخة | Defer | M2 | مؤجَّل | — |
| **F-097** | التصنيف: مدار/خارجي/معطّل/مكرر/غير متوافق | Keep | M3 | بلا أثر | — |
| **F-098** | **المحاور الأربعة متعامدة** | Keep | M3 | بلا أثر | — |
| **F-099** | كشف التكرار (SHA-1 + هوية المشروع) | Keep | M3 | بلا أثر | — |
| **F-100** | كشف تعارض الهاش | Keep | M3 | بلا أثر | — |
| **F-101** | مسار تحديث ميتافيزقا البروفايل | Keep | M1 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/db/instances.rs، core/src/instances/metadata.rs (+2) · كود: core/src/db/instances.rs، core/src/db/schema.rs (+4) |
### profiles

| المعرّف | البند | القرار | المرحلة | الأثر | الدليل |
|---|---|---|---|---|---|
| **F-102** | إنشاء بروفايل مخصّص | Keep | M1 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/db/instances.rs، core/src/instances/metadata.rs (+1) · كود: core/src/instances/registry.rs، core/src/journal/mod.rs |
| **F-103** | ربط مجلد موجود | Keep | M1 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/db/instances.rs، core/src/instances/registry.rs · كود: core/src/db/instances.rs، core/src/db/schema.rs (+2) |
| **F-104** | **عمود `source`** على النسخ | Keep | M1 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/db/instances.rs، core/src/db/mod.rs (+1) · كود: core/src/db/instances.rs، core/src/db/schema.rs (+3) |
| **F-105** | ~~`ownership_policy` مُنفَّذ~~ | Drop | — | متروك | — |
| **F-106** | معاينة إنشاء على مجلد موجود | Keep | M2 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/instances/registry.rs · كود: core/src/instances/registry.rs، src/app/pages/NewInstanceDialog.tsx |
| **F-107** | **تراجع إنشاء كامل** | Keep | M1 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/instances/registry.rs · كود: core/src/instances/registry.rs |
| **F-108** | تفعيل/تعطيل (إعادة تسمية) | Keep | M2 | بلا أثر | — |
| **F-109** | **تراجع تنفيذي عكسي** | Keep | M2 | بلا أثر | — |
| **F-110** | رفض الروابط الرمزية والتجاوز والتصادم | Keep | M1 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/instances/toggle/tests.rs |
| **F-111** | حذف بمشغّل أمان **بلا حذف دائم** | Keep | M2 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/instances/remove/tests.rs، src-tauri/src/commands/tests.rs · كود: cli/src/commands/remove.rs، core/src/instances/remove/mod.rs (+3) |
| **F-112** | كلمة تأكيد `REMOVE` للملفات الخارجية | Keep | M2 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/instances/remove/tests.rs · كود: core/src/instances/remove/mod.rs، src-tauri/src/commands.rs (+2) |
| **F-113** | نسخ احتياطي كامل للبروفايل | Keep | M2 | بلا أثر | — |
| **F-114** | استعادة بحفظ الوجهة القائمة جانباً | Keep | M2 | بلا أثر | — |
| **F-115** | **نسخ `saves` اختياري** | Keep | M2 | بلا أثر | — |
| **F-116** | مدير نسخ التحديث (لكل ملف) | Defer | later | مؤجَّل | — |
| **F-117** | إصلاح الميتافيزقا (خطة) | Defer | later | مؤجَّل | — |
| **F-118** | معاينة العوالم (قراءة فقط) | Keep | M3 | بلا أثر | — |
| **F-119** | استيراد عالم (نسخ فقط) | Defer | later | مؤجَّل | — |
| **F-181** | استيراد بيانات التطبيق القديم (باستود) | Keep | later | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/legacy/import_tests.rs، core/src/legacy/tests.rs (+1) · كود: core/src/legacy/import.rs، core/src/legacy/mod.rs (+6) · اختبار (ذكر): core/src/legacy/import_tests.rs، src-tauri/src/commands/legacy_tests.rs · وثائق: docs/adr/0003-scope-decisions.md، docs/adr/0006-legacy-import.md |
### updates

| المعرّف | البند | القرار | المرحلة | الأثر | الدليل |
|---|---|---|---|---|---|
| **F-120** | معاينة التحديث | Keep | M2 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/updates/tests.rs، src-tauri/src/commands/updates_tests.rs · كود: core/src/updates/mod.rs، src-tauri/src/commands.rs (+1) |
| **F-121** | **إعادة تحقق قبل التنفيذ** | Keep | M2 | بلا أثر | — |
| **F-122** | عقد السلامة (5 بنود) | Keep | M2 | بلا أثر | — |
| **F-123** | **نسخ احتياطي Yes/No/Cancel** | Keep | M2 | بلا أثر | — |
| **F-124** | تحديث بالتنزيل فقط | Keep | M2 | بلا أثر | — |
| **F-125** | مخطّط الإصدار المستهدف | Keep | M3 | بلا أثر | — |
| **F-126** | اقتراح هدف مشترك | Keep | M3 | بلا أثر | — |
| **F-127** | **تنظيف الهدف يترك المجهول** | Keep | M3 | بلا أثر | — |
| **F-128** | `installed_mods.json` مانيفست | Keep | M2 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/instances/record.rs، core/src/instances/tests.rs · كود: core/src/instances/tests.rs، core/src/updates/mod.rs |
| **F-129** | مسح المودات المحلية + استيراد | Keep | M3 | مُنفَّذ ومُشار إليه في الكود | كود: core/src/updates/mod.rs |
| **F-130** | **قصّ الميتافيزقا فقط** | Keep | M3 | بلا أثر | — |
| **F-131** | **التمييز «موجود» عن «مؤهَّل»** | Keep | M3 | بلا أثر | — |
| **F-132** | كاش لكل تشغيل في المخطّط | Keep | M3 | بلا أثر | — |
### storage

| المعرّف | البند | القرار | المرحلة | الأثر | الدليل |
|---|---|---|---|---|---|
| **F-133** | ترحيلات SQLite المرقّمة | Keep | M1 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/db/mod.rs · كود: core/src/db/schema.rs |
| **F-134** | **بوابة إصدار المخطط المستقبلي** | Keep | M1 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/db/mod.rs، src-tauri/src/commands/tests.rs |
| **F-135** | نسخة احتياطية قبل الترحيل | Keep | M1 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/db/mod.rs |
| **F-136** | **نشر النسخة بلا استبدال** | Keep | M1 | مُنفَّذ ومُشار إليه في الكود | كود: core/src/db/mod.rs |
| **F-137** | **fail-closed عند القفل** | Keep | M1 | بلا أثر | — |
| **F-138** | **تنقية رسائل الترحيل** | Keep | M1 | بلا أثر | — |
| **F-139** | **فشل قبل بناء النافذة** | Keep | M1 | بلا أثر | — |
| **F-140** | **WAL** | Keep | M1 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/db/mod.rs |
### operations

| المعرّف | البند | القرار | المرحلة | الأثر | الدليل |
|---|---|---|---|---|---|
| **F-141** | **سجلّ عمليات ذرّي (JSON)** | Keep | M1 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/instances/recovery.rs، core/src/journal/store/tests.rs (+2) · كود: core/src/downloader/batch.rs، core/src/downloader/mod.rs (+1) |
| **F-142** | انتقالات مُقيّدة للعمليات والعناصر | Keep | M1 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/journal/store/tests.rs، core/src/journal/tests.rs · كود: core/src/journal/mod.rs |
| **F-143** | **كشف مجلد عمليات غير منتهية** | Keep | M1 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/journal/store/tests.rs، core/src/journal/tests.rs · كود: core/src/journal/mod.rs |
| **F-144** | **القفل عبر العمليات** | Keep | M2 | بلا أثر | — |
| **F-145** | مركز العمليات (قراءة فقط) | Keep | M2 | بلا أثر | — |
| **F-146** | **تقييم الأدلة — أعداد فقط** | Keep | M3 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/instances/recovery.rs، core/src/modpack/recovery_tests.rs (+1) · كود: core/src/instances/recovery.rs، src-tauri/src/commands/pack.rs (+5) · اختبار (ذكر): src-tauri/src/commands/pack_tests.rs |
| **F-147** | **تعليق كـ«مُهجر»** | Defer | later | مؤجَّل | — |
### logging

| المعرّف | البند | القرار | المرحلة | الأثر | الدليل |
|---|---|---|---|---|---|
| **F-148** | تنقية الأسرار من السجلات | Keep | M1 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/logging/sanitize.rs · كود: core/src/logging/sanitize.rs |
| **F-149** | **السجل المحلي كامل وغير منقّى** | Keep | M1 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/logging/mod.rs · كود: core/src/logging/mod.rs |
| **F-150** | **فشل مغلق بلا مخرج خام** | Keep | M1 | بلا أثر | — |
### settings

| المعرّف | البند | القرار | المرحلة | الأثر | الدليل |
|---|---|---|---|---|---|
| **F-151** | إعدادات محفوظة (21 مفتاحاً) | Keep | M2 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/db/mod.rs، src-tauri/src/commands/diagnostics_tests.rs · كود: src-tauri/src/commands.rs، src-tauri/src/dto.rs (+2) |
| **F-152** | سجلّ التنزيلات | Keep | M2 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/db/download_history.rs، src-tauri/src/commands/channel_tests.rs (+1) · كود: core/src/db/download_history.rs، core/src/db/schema.rs (+4) · اختبار (ذكر): src-tauri/src/commands/channel_tests.rs، src-tauri/src/commands/history_tests.rs |
| **F-153** | **فهرس على سجلّ التنزيلات** | Keep | M2 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/db/download_history.rs · كود: core/src/db/download_history.rs، core/src/db/schema.rs (+3) |
| **F-154** | المفضّلة | Defer | later | مؤجَّل | — |
### import/export

| المعرّف | البند | القرار | المرحلة | الأثر | الدليل |
|---|---|---|---|---|---|
| **F-155** | تصدير بروفايل zip | Keep | M2 | بلا أثر | — |
| **F-156** | **قائمة الاستبعاد** | Keep | M2 | مُختبَر بمعرّف المواصفة | اختبار بمعرّف: core/src/modpack/export_tests.rs · كود: core/src/modpack/export.rs · اختبار (ذكر): core/src/modpack/export_tests.rs |
| **F-157** | `export_manifest.json` | Keep | M2 | بلا أثر | — |
| **F-158** | **استيراد ميتا فقط بلا استخراج** | Keep | M2 | بلا أثر | — |
| **F-159** | تصدير قائمة المودات (MD/CSV/TXT) | Drop | — | متروك | — |
| **F-160** | استيراد/تصدير JSON | Drop | — | متروك | — |
| **F-161** | اختيار هدف عالم لقاعدة البيانات | Defer | later | مؤجَّل | — |
### ui

| المعرّف | البند | القرار | المرحلة | الأثر | الدليل |
|---|---|---|---|---|---|
| **F-162** | شريط تنقّل + صفحات مكدّسة | Keep | M2 | بلا أثر | — |
| **F-163** | لوحة **Home** | Drop | — | متروك | — |
| **F-164** | صفحة Discover | Keep | M2 | بلا أثر | — |
| **F-165** | جدول نتائج (13 عموداً) | Keep | M2 | بلا أثر | — |
| **F-166** | ترقيم «Load more» | Keep | M2 | بلا أثر | — |
| **F-167** | **سجلّ تنقّل 32 مدخلاً** | Drop | — | متروك | — |
| **F-168** | **مراقب نظام الملفات (تلميح)** | Keep | M2 | بلا أثر | — |
| **F-169** | لوحة تنقّل لوحة المفاتيح (cmdk) | Keep | M2 | بلا أثر | — |
| **F-170** | الثيم واللون | Keep | M2 | بلا أثر | — |
| **F-171** | **اختصارات لوحة المفاتيح** | Keep | M2 | مُنفَّذ ومُشار إليه في الكود | كود: src/app/CommandPalette.tsx |
| **F-172** | شريط عنوان مخصّص (إطار مخفي) | Drop | — | متروك | — |
| **F-173** | RTL + i18n | Keep | M2 | بلا أثر | — |
| **F-174** | تخطيط متجاوب | Keep | M2 | بلا أثر | — |
### `licensing.py` · `main_window.py:14141` · `packaging/license/*`

| المعرّف | البند | القرار | المرحلة | الأثر | الدليل |
|---|---|---|---|---|---|
| **F-175** | **الترخيص والتفعيل دون اتصال** | — | **قرارك: Drop نهائي.** تطبيق مجاني — لا نظام ترخيص ولا بصمة جهاز ولا نافذة تفعيل | إشارة في الوثائق فقط | وثائق: docs/adr/0003-scope-decisions.md |
### `packaging/**`

| المعرّف | البند | القرار | المرحلة | الأثر | الدليل |
|---|---|---|---|---|---|
| **F-176** | **بناء PyInstaller + Inno + AppImage** | — | Tauri bundler يحلّها (`foundations.md` §7) | بلا أثر | — |
### `PHASE16_5/6/7`

| المعرّف | البند | القرار | المرحلة | الأثر | الدليل |
|---|---|---|---|---|---|
| **F-177** | **نماذج العامة/الخاصة** | — | متعلّق بـF-175 | بلا أثر | — |
### `diagnostics.py`

| المعرّف | البند | القرار | المرحلة | الأثر | الدليل |
|---|---|---|---|---|---|
| **F-178** | **تشخيص ذاتي + قوالب بناء** | — | يستحق فقط مع الترخيص | بلا أثر | — |
### `config.py:27`

| المعرّف | البند | القرار | المرحلة | الأثر | الدليل |
|---|---|---|---|---|---|
| **F-179** | **وضع محمول** | — | `directories` يكفي | مُنفَّذ ومُشار إليه في الكود | كود: core/src/paths.rs |
### `.github/workflows/`

| المعرّف | البند | القرار | المرحلة | الأثر | الدليل |
|---|---|---|---|---|---|
| **F-180** | **CI لاختبارات Python** | — | لا CI أصلاً؛ يكتب سطر واحد لـCargo | بلا أثر | — |

---

**قراءة الأرقام:** `مُختبَر بمعرّف المواصفة` = المعرّف داخل اسم اختبار (أو عنوانه) · `مُنفَّذ ومُشار إليه في الكود` = المعرّف حرفيًّا في كود إنتاج · `مذكور في الاختبارات` = حرفيًّا في ملف اختبار بلا معرّف في الاسم · `إشارة في الوثائق فقط` · `بلا أثر` = لا ذكر على الإطلاق.
