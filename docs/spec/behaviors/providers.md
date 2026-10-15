# behaviors/providers.md — المزوّدون وحدود المعدل وإعادة المحاولة

> **المرجع:** `D:\latest_project\app\providers\base.py` · `app\providers\modrinth.py` · `app\providers\modrinth_mapping.py` · `app\api\modrinth_client.py` · `tests\qa\test_modrinth_api_resilience.py`
> **قاعدة القراءة:** السلوك موصوف، والكود منقول. كل رقم أدناه مقروء من المصدر.

---

## 0. CurseForge: غير موجود إطلاقاً (إجابة قاطعة)

بحث حالة-غير-حالة (`curseforge|cf_api_key|curse`) عبر `app/` يعطي **4 نتائج كلها إيجابيات كاذبة** (`recurse_resolved_dependencies` في `app/services/resolver.py:…`). وعلى `tests/` النتائج الأربع الوحيدة هي ثوابت تنقية أسرار في `tests/qa/test_log_sanitizer.py:217,218,226,227`.

**لا عميل، لا مزوّد، لا إعداد مفتاح، لا تحويل بصمات، لا اختبارات.**

المشروع القديم يسمّي نفسه «Modrinth Bulk Downloader» وهو **Modrinth فقط**. CurseForge **ميزة جديدة بالكامل** لا نحمل منها أي سلوك قديم — لذا هي **غير مُتحقَّق** بطبيعتها، ولا يجوز افتراض أي منها.

> **الوثائق المصاحبة لذلك:** `docs/release/handoff_bundle_docs/HANDOFF.md:74` و`…\PROJECT_MAP.md:162` يقولان صراحةً إن CurseForge غير منفَّذ.

---

## 1. منفذ المزوّد — `app/providers/base.py`

`ContentProvider` هو `@runtime_checkable Protocol` (سطر 17-18). `ContentProviderError(RuntimeError)` هو الأساس المشترك (سطر 13).

### 1-1. السطح المُتحقَّق منه (12 عملية + خاصية)

| العضو | التوقيع | النتيجة |
|---|---|---|
| `provider_id` | `@property` | `str` |
| `get_game_versions` | `(self)` | `list[str]` |
| `get_loader_tags` | `(self)` | `list[ContentLoaderTag]` |
| `get_categories` | `(self)` | `list[ContentCategoryTag]` |
| `get_side_types` | `(self)` | `list[str]` |
| `get_project_types` | `(self)` | `list[str]` |
| `search_projects` | `(query, game_version, loader, *, project_type="mod", category="", extra_categories=None, client_side="", server_side="", limit=20, offset=0, index="relevance")` | `tuple[list[ContentProject], int]` = (النتائج، إجمالي النتائج) |
| `get_project` | `(project_id_or_slug)` | `ContentProject` |
| `get_version` | `(version_id)` | `ContentVersion` |
| `get_project_versions` | `(project_id_or_slug, game_version, loader, *, include_changelog=False, filter_compatible=True)` | `list[ContentVersion]` |
| `get_best_version` | `(project_id_or_slug, game_version, loader, *, allow_beta=True, allow_alpha=False)` | `ContentVersion \| None` |
| `get_versions_from_hashes` | `(hashes, algorithm="sha1")` | `dict[str, ContentVersion]` بمفتاح = الهاش المُرسَل **مُحوَّلاً لأحرف صغيرة** |
| `get_latest_versions_from_hashes` | `(hashes, *, loaders, game_versions, algorithm="sha1")` | `dict[str, ContentVersion]` |

### 1-2. ملاحظات تصميمية مهمة

- **لا يوجد في المنفذ أي عملية تنزيل/نقل.** `ContentFile.url` يُمرَّر مباشرة إلى `DownloadManager`. قرار موثّق في `docs/superpowers/specs/2026-07-12-content-provider-modrinth-adapter-design.md:43`.
- `ModrinthClient` يعرّض طريقتين **خارج** المنفذ: `get_loaders(project_type="mod")` (سطر 172) وغلاف قديم `search_mods(...)` (سطر 308).
- `ModrinthProviderAdapter` (`app/providers/modrinth.py`) **مُفوِّض حرفي 1:1**، `provider_id = "modrinth"`، يقبل `client` اختيارياً.
- `tests/qa/test_content_provider.py::test_protocol_has_only_the_verified_application_lookup_surface` **يثبّت** هذا السطح من 12 عملية — اختبار يعاقد، لا اختبار سلوك.

---

## 2. عميل Modrinth — `app/api/modrinth_client.py`

### 2-1. الثوابت

| البند | القيمة | الدليل |
|---|---|---|
| الأساس | `https://api.modrinth.com/v2` | `app/config.py:17` |
| `User-Agent` | `mhmhamed8/modrinth-bulk-downloader/0.14.0rc1 (local desktop app)` | `app/config.py:10,11,15`، يُضبط مرة على الجلسة |
| الترويسة الإضافية | `Accept: application/json` | السطر 92 |
| المهلات | `(10.0, 45.0)` = (اتصال، قراءة) | افتراضي، يُمرَّر لـ`session.request` |

> ⚠️ **خلل قائم:** `app/config.py:13-14` يحمل تعليقاً يقول «استبدل الجزء المتعلق بالتواصل»، لكن السلسلة الفعلية **لا تحتوي بريداً ولا رابط تواصل**. `AGENTS.md` §5.11 يشترط `User-Agent` وصفياً **بمعلومات تواصل** — فهذا انحراف يجب عدم تكراره.

### 2-2. عدد المحاولات وما الذي يُعاد

```
attempts = 2 if (retry_once_on_rate_limit and read_only) else 1
```

- **الحد الأقصى محاولتان، دائماً.** لا backoff أُسّي في العميل.
- `read_only` (السطر 105) = الطريقة في `{GET, HEAD}` **أو** (`POST` و المسار في `{"/version_files", "/version_files/update"}`).
- أي `POST` أو `DELETE` آخر = **محاولة واحدة**.
- `time.sleep(1.0)` قبل المحاولة الوحيدة الثانية، وبعدها فقط: `RequestsTimeout` أو `RequestsConnectionError` (السطران 115-118).
- أي `RequestException` أخرى → **تُرفع فوراً ولا تُعاد** (السطران 125-126).

### 2-3. الحالات المعاد المحاولة عليها

`{429}` ∪ `{500, 502, 503, 504}` فقط (السطر 130). أي 5xx آخر (مثل 501 أو 507) → `ModrinthApiError` عادي.

### 2-4. تحليل `Retry-After` — `_server_retry_delay` (الأسطر 55-86)

**ترتيب الترويسات:** `"Retry-After"` ثم `"X-Ratelimit-Reset"`، وأول واحد صالح يفوز.

| الشكل | القبول | النتيجة |
|---|---|---|
| ثوانٍ خام | يطابق كامل `[0-9]+(\.[0-9]+)?` | `float` |
| تاريخ HTTP | **على `Retry-After` فقط** (لا على `X-Ratelimit-Reset`)، ويشترط `_HTTP_DATE.fullmatch` | يُحلّل |
| قيمة مشوّهة | تُتجاهل وتنتقل للترويسة التالية | — |
| لا شيء صالح | `None` | — |

**ثلاثة أشكال تاريخ مقبولة:** RFC 1123 `Thu, 01 Jan 1970 00:00:03 GMT` · RFC 850 `Thursday, 01-Jan-70 00:00:08 GMT` · asctime `Thu Jan  1 00:00:08 1970`.

**قاعدة ال50- سنة الدوّارة** (الأسطر 68-78) — بديل مقصود لتقسيمLibrary على 1969/2068:

```
year = now.year + (retry_at.year % 100 - now.year) % 100
if resulting_datetime > now + 50 years:  year -= 100
```

**حدود معتمدة في الاختبارات:**
- `now=2026-09-11`: `Wednesday, 11-Sep-69` و `Friday, 11-Sep-76` → **مستقبل** (`reset_seconds > 120`).
- `Saturday, 12-Sep-76` → **ماضٍ** (`sleep(0)`).
- القيم المشوّهة تُختبر: `'garbage'`, `'-2'`, `'NaN'`, `'Infinity'`, `''`, `'1_0'`, `'…GMT INVALID'`, `'…GMT junk'`.
- تاريخ في الماضي → `0`.
- `'10.01'` (> 10) → **خطأ فوري بلا نوم**.

### 2-5. ميزانية الانتظار — القاعدة الأهم في هذا الملف

```
delay = 60.0 (لـ 429) أو 1.0 (لـ 5xx) إذا لم تُحلَّل ترويسة
if not may_retry or delay > 10:      ← تفشل فوراً، بلا نوم
    429 → ModrinthRateLimitError(ceil(delay))   يحمل .reset_seconds
    5xx → ModrinthApiError("… Try again later.")
else:
    sleep(delay); أعد مرة واحدة
```

> **لا يُقصَّ الانتظار الطويل لإعادة المحاولة قبل انتهائه** (تعليق السطر 134). التطبيق **لا ينام أبداً أكثر من 10 ثوانٍ**؛ يرفض ويخبر المتصل كم يجب أن ينتظر.
> `response.close()` في `finally` على كل محاولة (الأسطر 150-151) — ويؤكده عدة اختبارات.
> `raise ModrinthApiError("Modrinth request attempts exhausted.")` في السطر 153 **غير قابل للوصول عملياً** لأن فرع `may_retry=False` يرفع أولاً.

### 2-6. تدهور التوافق عند HTTP 400

`_ModrinthBadRequest(ModrinthApiError)` يُرفع في السطر 141. `search_projects` (الأسطر 282-301) يلتقطه **فقط** حين `project_type in {"plugin", "datapack"}`؛ غير ذلك يُعاد رفعه. عند الالتقاط يُعاد الطلب بـ`project_type_override="mod"` و`loader=loader or project_type` — أي **تُحذف واجهة `project_type:<x>`** ويُبحث في المودات بدل ذلك. خطأ 400 ثانٍ يتصاعد كـ`ModrinthApiError`.

> **اختبار dissected — انتبه:** `test_bad_request_fallback_preserved_without_transport_fallback` ينصّ أن النصف الثاني «لا يتجاوز انتظار الخادم»؛ والنصف الأول «يحفظ التدهور». لكن مع `Timeout` دائم، الطلبان هما **إعادة محاولة النقل** للطلب الأول، فلا يُبلغ التدهور أصلاً. **الاسم يَعِد بأكثر مما يختبر.**

### 2-7. تحليل التواريخ — نقطة عمى

`date_published` حقل **`str` خام** في `app/domain/content.py:64`، و`get_best_version` يرتّب **كنص عادي**:
`sorted(..., key=lambda v: v.date_published, reverse=True)` (السطر 444).

> لا يوجد تحليل ISO-8601 إطلاقاً. رتّب التواريخ نصّاً = خطأ صامت عند اختلاف الأطوال/المناطق الزمنية. **غير مُتحقَّق** إن كان Modrinth يعيد صيغة موحّدة دائماً.

### 2-8. حدود المعدل

**لا يوجد تحديد معدل من جهة العميل إطلاقاً.** لا خانزنة، لا دلو، لا حدّ أدنى بين الطلبات. `grep -i "ratelimit|Retry-After"` عبر `app/` يعطي 8 نتائج، كلها في `artifact_verifier.retry_delay_seconds` وفي المعالجة التفاعلية أعلاه. **التعامل تفاعلي بحت مع 429 من الخادم.**

### 2-9. رابط التنزيل الفارغ — مسار كامل

| المرحلة | السلوك | الدليل |
|---|---|---|
| التحويل | `url = file_data.get("url") or ""` — الغياب يصير `""` بصمت | `modrinth_mapping.py:46` |
| النموذج | `ContentFile.url` **بلا مُتحقِّق**، معامل موضعي | `domain/content.py:39` |
| الإصدار | `version_from_modrinth` **لا يُسقط** الملف، و`primary_file()` يعيد أول ملف `primary` (أو `files[0]`) **بregardless** من الرابط | `domain/content.py:69-75` |
| التصفية | `get_project_versions` يفلتر بـ`version.primary_file() is not None` فقط — **بلا فحص رابط** | السطر 419 |
| العميل | لا يفحص الروابط إطلاقاً | — |

**التنفيذ يقع في الطبقة الأدنى:**

| المكان | الاختبار |
|---|---|
| `app/ui/workers.py:1273` | `file is None or not file.url` |
| `app/ui/main_window.py:9157` | نفس |
| `app/services/batch_install_plan.py:84` | نفس |
| `app/services/safety_checks.py:70` | نفس |
| `app/services/mrpack_service.py:386` | `not file.url or not _valid_download_url(file.url)` |
| `artifact_verifier.validate_download_url("")` | `urlsplit("").hostname is None` → `ValueError("Download URL must include a host.")` |

> **الدلالة على CurseForge:** الحالة «`downloadUrl` فارغ لأن المؤلف منع التحميل الخارجي» **موصوفة في `foundations.md` §4.4 كسلوك معروف**، لكنها **غير مُنفَّذة في أي مكان** — لا اختبار، لا كود. **غير مُتحقَّق.**

### 2-10. الخصوصية في الأخطاء

رسائل `ModrinthApiError` **لا تحوي جسم الاستجابة أبداً**. هذا مُختبَر صراحةً: `test_modrinth_api_resilience.py:42,99,107,240` يؤكد أن النص `"private"` (نص الاستجابة المزيفة) لا يظهر في الاستثناء.

---

## 3. تصنيف الأخطاء (النقل إلى `IpcError`)

| النوع | الأساس | رسالة الواجهة (مقتطف) |
|---|---|---|
| `ContentProviderError` | `RuntimeError` | الأساس |
| `ModrinthApiError` | `ContentProviderError` | `"Modrinth API error {status}."` · `"… Try again later."` · `"Modrinth returned invalid JSON."` · `"Modrinth did not respond in time…"` · `"Cannot connect to Modrinth…"` · `"Network error while contacting Modrinth."` · `"Choose a specific loader/platform before checking updates."` · `"Choose a specific Minecraft version before checking updates."` |
| `_ModrinthBadRequest` | `ModrinthApiError` | داخلي فقط |
| `ModrinthRateLimitError` | `ModrinthApiError` | `"… rate limit exceeded. Try again in {reset_seconds} seconds."` + `.reset_seconds: int` |

> **تحذير نقل:** كل هذه **نصوص إنجليزية حرفية داخل كود الخدمة**. لا رموز أخطاء ولا مفاتيح i18n في هذه الطبقة. `AGENTS.md` §5.3 يفرض `IpcError { code, message }` — فالنقل يحتاج **تصنيفاً حقيقياً** لا ترجمة سلاسل.

---

## 4. حالات حدّية مرصودة في الكود والاختبارات

| الحالة | السلوك | الدليل |
|---|---|---|
| قيمة انتظار مشوّهة + `X-Ratelimit-Reset` صالح طويل | **`Reset` يفوز**، ولا نوم، وخطأ واحد | `test_malformed_retry_after_cannot_override_valid_long_reset` |
| `Retry-After` = 120 مع `Reset` = 0 | **`Retry-After` يفوز** → `reset_seconds == 120`، بلا نوم | `test_long_server_wait_is_not_clamped_to_early_retry` |
| خطأ نقل ثم خطأ نقل | طبيعي إلى `ModrinthApiError`، `call_count == 2`، بلا تسريب | `test_second_transport_error_is_normalized` |
| `retry_once_on_rate_limit=False` | محاولة واحدة، بلا نوم | `test_disabled_and_exhausted_retries_are_bounded` |
| جسم فارغ | يُعيد `None`، و`json()` **لا تُستدعى** | `test_head_empty_and_post_retry_arguments` |
| 400 على `plugin`/`datapack` مع انتظار 120 على البديل | **محاولة واحدة، بلا نوم** — التدهور لا يتجاوز بوابة الانتظار | `test_public_search_fallback_does_not_bypass_server_wait` |

## 5. اختبارات تحتاج تضييقاً (flags)

| الاختبار | المشكلة |
|---|---|
| `test_bad_request_fallback_preserved_without_transport_fallback` | النصف الثاني لا يختبر ما ينصّ عليه اسمه (يبلغ إعادة محاولة النقل بدل التدهور) |
| `test_wait_boundaries_and_obsolete_http_dates` | يحزم 4 صيغ تاريخ + حالتا حالة HTTP في اختبار واحد |
| `test_second_attempt_error_variants_and_disabled_http_retries` | يحزم 3 سيناريوهات مستقلة |

**عوامل:** كل اختبارات `ModrinthApiResilienceTests` **مُحاكاة بالكامل** (لا مقابس، لا قرص). لا اعتماد على الشبكة ولا نظام الملفات.

## 6. غير مُتحقَّق

| البند |
|---|
| هل غياب معلومات التواصل في `User-Agent` مخالف لسياسة Modrinth فعلياً |
| أي سلوك لـ CurseForge (غير موجود) |
| هل `date_published` من Modrinth موحّد الصيغة دائماً |
| نتائج التشغيل — لم أُشغّل أي اختبار (تكليف قراءة فقط) |