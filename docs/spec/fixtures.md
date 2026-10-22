# fixtures.md — ملفات واستجابات قابلة لإعادة الاستخدام

> **المصدر:** فحص متكرر شامل لـ `D:\latest_project` (خارج `.git`) بحثاً عن `*.jar` · `*.mrpack` · `*.zip` · `*.db` · `*.sqlite*` · `*.pem` · `profile.json` · `modrinth.toml` · `modrinth.index.json` · `installed_mods.json`.
> **لا أسرار ولا بيانات شخصية في هذا الملف.** أي قيمة تشبه سرّاً نُقّت بـ`<REDACTED>`.

---

## 1. ⚠️ النتيجة الأهمّ: **لا توجد ملفات ثابتة على الإطلاق**

| الامتداد | العدد في المستودع |
|---|---|
| `*.jar` | **0** |
| `*.mrpack` | **0** |
| `*.zip` (أرشيف اختبار) | **0** |

**كل الأرشيفات تُبنى داخل `TemporaryDirectory` وقت التشغيل.**

> ### ماذا يعني هذا؟
> **لاFixtures جاهزة للنقل.** كل اختبار اصطناعي يُولّد مدخلاته. **المهمة الحقيقية ليست نسخ ملفات — بل كتابة مولّدات مكافئة.**
> وهذا في الواقع **أفضل**: لا التزام بحجم أو ترخيص أي مود حقيقي.

---

## 2. ما وُجد فعلاً — وكله **ليس مدخل اختبار**

| المسار | الحجم | ما هو |
|---|---|---|
| `pyproject.toml` | **356 B** | بيانات المشروع: PySide6-Essentials ≥6.6.0 · requests ≥2.31.0 · cryptography ≥42,<47 · ruff line-length 100 |
| `.tmp\p14-self-check-1783938870223\data\app.sqlite3` | **8,192 B** | **أثر متبقٍ** من تشغيل `--self-check`. مخطط v2. **لا يشير إليه أي اختبار** |
| `.tmp\p14-self-check-1783938870223\logs\diagnostics_self_check_last.txt` | **175 B** | مخرجات الفحص الذاتي |
| `dist\…\certifi\cacert.pem` | 236,095 B | حزمة جذور CA العامة (PyInstaller) — **جذور عامة فقط، لا أسرار** |
| `build\…\base_library.zip` · `dist\…\base_library.zip` | 1,388,172 B | مكتبة بايثون القياسية (PyInstaller) |
| `artifacts\portable\…\windows-onedir.zip` | **48,998,964 B** | مُخرَج إصدار |
| `artifacts\portable\…\windows-portable-data.zip` | **49,002,587 B** | مُخرَج إصدار |
| `tests\qa\test_import_export_resolver_contracts.py.tmp` | **0 B** | ملف شارد بجانب وحدة اختبار. **غير مُجمَّع** |

### 2-1. محتوى `diagnostics_self_check_last.txt`

```
self_check: failed: Database migration for app.sqlite3 failed during
version-check: schema version 2, which is newer than supported version 1.
The database was not modified.
```

> **لا أسرار ولا أسماء مستخدمين.** ✅ آمن للاقتباس كـfixture **مثالي لاختبار بوابة الإصدار المستقبلي**.

---

## 3. كيف تُبنى الاستجابات	http **الاصطناعية

> **لا wiremock، لا monkeypatching لطبقة نقل، لا تسجيلات HAR.**

### 3-1. حقن الجلسة — `test_modrinth_api_resilience.py`

```python
patch.object(client.session, "request")     # ينتج Mock(status_code, headers, …)
setUp: patch("app.api.modrinth_client.time.sleep")
```

صانع `response()` يبني `Mock` بـ `.json()` · `.content` · `.text = 'private response payload'` · `.close()` قابلة للحقن.
**يغطّي كل توليفات الحالة/الترويسة/المحاولة.**

> **في Rust:** `wiremock` (المذكور في `AGENTS.md` §4) يعطي الشيء نفسه **وأقوى** — يعيد الحالة والترويسات والجسم الحقيقي.

### 3-2. خادم HTTP حقيقي على loopback — `test_verified_artifact_pipeline.py`

```python
ThreadingHTTPServer(("127.0.0.1", 0), _ArtifactHandler)   # منفذ يختاره النظام
```
يقدّم jar اصطناعي (`_jar_bytes()`: `META-INF/MANIFEST.MF` + `fabric.mod.json`) عبر مسارات مبرمجة:

| المسار | السلوك |
|---|---|
| `/artifact.jar` | 200 عادي |
| `/retry-429.jar` | 429 مرة واحدة ثم 200 |
| `/retry-503.jar` | 503 مرة واحدة ثم 200 |
| `/html.jar` | 200 بنوع `text/html` |
| `/not-found.jar` | 404 |
| `/no-range.jar` | **200 دائماً رغم ترويسة `Range`** |
| أي مسار | يدعم 206 مع `Range` |

عدادات على مستوى الصنف: `requests_by_path` · `range_headers`.

> **لماذا لم يكتفوا بالحقن؟** لأن إعادة التوجيه و`Range` والـ`Content-Type` تحتاج مقبساً حقيقياً. **في Rust: `wiremock` يعالج الحالة والترويسات؛ السلوك غير القياسي يحتاج خادماً صغيراً أو اعترافاً بأن اختبار `no-range` يُكتب يدوياً.**

### 3-3. بدائل مكتوبة يدوياً — بلا HTTP إطلاقاً

| البديل | الاستخدام |
|---|---|
| `RecordingClient` | `__getattr__` يُعيد دالة تسجيل فوق `calls` / `results` / `errors` |
| `StatefulFakeProvider` | قاموس: `projects` · `versions` · `project_versions` · `best_versions` · `latest_versions` · `search_results` + سجلّ `calls` |
| `FakeModrinthClient` | الشكل نفسه — **يُستخدم في كل اختبارات المبدّل الـ166** |
| `_FakeDownloadManager` | يحترم `url_validator` و`state_callback`، خطّاف `before_success` لحقن السباقات/التلاعب |
| `_FakeUser32` · `_FakeApplication` · `_FakeNullIcon` · `_FakeStyle` · `_FakeSignal` · `_FakeOperationCenterDialog` | بدائل Qt/HAL |

> **⭐ `FakeModrinthClient` هو أقوى استثمار في المشروع كله:** يُسجّل **كل** نداء في `self.calls`، والادّعاءات كلها على **شكل** النداء لا على القيمة. **هذه التقنية يجب أن تُبنى أولاً** في `core/tests/` — `trait ContentProvider` يُنفَّذ بـ `calls: RefCell<Vec<(String, Args)>>`.

### 3-4. أرشيفات مبنية بـ`zipfile`

| المولّد | ينتج |
|---|---|
| `_zip_bytes(entries, compression=…)` | أسماء أعضاء اعتباطية — `../evil.txt` · `/absolute.txt` · `C:/drive.txt` · `folder//alias.txt` · `folder/./alias.txt` · `folder/trailing. /file.txt` · `folder/CON.txt` · أزواج NFC/NFD |
| أعضاء روابط رمزية | `ZipInfo.create_system = 3` و`external_attr = (stat.S_IFLNK \| 0o777) << 16` |
| أعضاء مشفّرة | `infolist()[0].flag_bits \|= 0x1` |
| قنابل ضغط | `b"0" * 4096` · `b"0" * (2*1024*1024)` |
| أعضاء ضخمة | `MRPACK_INDEX_MAX_BYTES + 1` · `PROFILE_EXPORT_JSON_MAX_BYTES + 1` |
| `_jar_bytes()` | jar اصطناعي للاختبار |
| `_index_payload()` · `_file_entry()` · `_write_pack()` | `.mrpack` اصطناعي كامل: `formatVersion` · `game` · `versionId` · `name` · `summary` · `files` · `dependencies` · `env.client ∈ {required, optional, unsupported}` · **sha1/sha512 حقيقيان** · الطبقات الثلاث |
| `_jar_with(path, entries)` | `fabric.mod.json` · `META-INF/mods.toml` · `pack.mcmeta` · `shaders/composite.fsh` · `My Pack/data/.../load.mcfunction` |

**الحقول الوصفية (`profile.json` · `mmc-pack.json` · `installed_mods.json`) تُكتب بـ`json.dumps` داخل أجسام الاختبارات.**

---

## 4. ما يستحق **بناءه** كـfixtures حقيقية في Rust

| # | الـfixture | يغطي | الأولوية |
|---|---|---|---|
| F-1 | **مولّد jar بالمتحكم** — `build_jar(entries: &[(&str, &[u8])]) -> Vec<u8>` | التحقق، بيانات المود، التصنيف، الأمان | **P0** |
| F-2 | **jar Fabric** — `fabric.mod.json` ببيانات معروفة | `local_mod_metadata` · `classify_local_resource_type` | **P0** |
| F-3 | **jar Forge** — `META-INF/mods.toml` بنطاق `versionRange` | نفس | **P0** |
| F-4 | **jar Quilt** — `quilt.mod.json` | نفس | **P1** |
| F-5 | **jar NeoForge** — `META-INF/neoforge.mods.toml` | **حارس التعارض الكاذب #1** | **P0** |
| F-6 | **jar شيدر** — `pack.mcmeta` + `shaders/composite.fsh` | تصنيف + تدرّج اللودر | **P1** |
| F-7 | **مولّد أرشيف آ hostile** — `build_hostile_archive(kind)` | 12 اختبار أمان الأرشيف | **P0** |
| F-8 | **`.mrpack` كامل** — فهرس + طبقات overrides + هاشات حقيقية | 31 اختبار مُثبِّت | **P1** |
| F-9 | **سجلّ HTTP معاد بناؤه** — كل تركيبات 429/5xx/Retry-After | 16 اختبار مزوّد | **P1** |
| F-10 | **قاعدة بيانات مُرمَّزة كـ bytes** — v0 / v1 / v2 / قاعدة أحدث | 36 اختبار ترحيل | **P0** |
| F-11 | **`diagnostics_self_check_last.txt`** (175 B، أعلاه) | رسالة بوابة الإصدار | **P0** |
| F-12 | **سجلّ عمليات بحالات** — planned/running/completed/failed/cancelled × كل طور عنصر | 34 اختبار سجلّ | **P1** |
| F-13 | **`profile.json` v1 و v2** | الاستيراد/التصدير | **P1** |
| F-14 | **jar بترويسة `.jar.disabled`** وjar داخل `disabled_mods/` | التفعيل/التعطيل | **P2** |
| F-15 | **`installed_mods.json`** بسجلّات مدار/خارجي/باهت | التصنيف | **P2** |

### 4-1. قيود القبول لكل fixture

| القاعدة | التطبيق |
|---|---|
| **بلا أسرار** | كل القيم اصطناعية |
| **بلا بيانات شخصية** | استخدم `alice` · `example.com` — **لا تنسخ أسماء حقيقية** |
| **بلا مواد محمية حقوقياً** | كل الأرشيفات مبنية برمجياً |
| **حجم صغير** | كل واحد < 100 KB |
| **قابل للتوليد** | **مولّد** لا ملف ثابت — نفس مبدأ المشروع القديم، **وهو healthiest** |

---

## 5. تدقيق الأسرار والبيانات الشخصية ✅

| الفئة | النتيجة |
|---|---|
| مفاتيح CurseForge | **لا شيء** |
| رموز Modrinth | **لا شيء** |
| مفاتيح ترخيص | **لا شيء** |
| أسماء مستخدمين للبيت | **لا شيء** |
| تواريخ CA | **جذور عامة فقط** |

**القيم السرّية الوحيدة** هي ثوابت اختبار مقصودة لإثبات التنقية — **مُدرجة كاملة في `behaviors/logging.md` §5**، وكلها اصطناعية.

**هويّات اصطناعية:** Alice · Alice Example · alice@example.com · `C:\Users\Alice` · /home/alice

**مادة تشفيرية:** بلا شيء حقيقي — `"a"*40` · `"b"*128` · UUIDs ثابتة · JWT ثابت اصطناعي.

> ⚠️ `LAPTOP-MUXEROUS` يظهر في `docs/baseline.md:41` — **اسم جهاز حقيقي من قياس المؤلف.** **لا تنقله**؛ استعمل `TEST-DEVICE`.

---

## 6. ملفات `fixtures/` في المستودع الجديد

```
core/tests/fixtures/
  ├─ mod/            F-1..F-6  مولّدات jar
  ├─ archive/        F-7       مولّدات أرشيف آ hostile
  ├─ modpack/        F-8       .mrpack
  ├─ db/             F-10      قواعد بيانات مُرمَّزة
  ├─ journal/        F-12      سجلّ عمليات
  └─ json/           F-13      profile.json v1/v2 · installed_mods.json
     └─ curseforge/            تسجيلات استجابات CurseForge الحقيقية (§7)
```

**كل المولّدات دوال في `core/tests/fixtures/mod.rs` — لا ملفات ثنائية في git.**
(يبقى المستودع خفيفاً، ولا مشاكل ترخيص، ولا اختلافات ثنائية عبر المنصّات.)

> **استثناء واحد:** `json/curseforge/` ملفات **JSON نصوص** مُسجَّلة من الـAPI الحقيقي — ليست مولّدات.
> قواعدها في §7-2.

---

## 7. تسجيلات استجابات CurseForge الحقيقية

> **قرار مُغلق:** نعم، نسجّل استجابات الـAPI الحقيقية — **بشروط صارمة** أدناه.
> هذا يُغلق السؤال المفتوح في §8.

### 7-1. لماذا كانت لازمة

`docs.curseforge.com` غير متاح، وعميل Python القديم
`app/api/curseforge_client.py` **يمرّر JSON كما هو** (يقرأ خمسة مفاتيح في الملف كله). فهو يثبت
**طبقة النقل** — base URL · ترويسة `x-api-key` · مسارات النقاط · أسماء معاملات الاستعلام ·
غلاف `{data, pagination}` · جداول معرّفات اللودر · عقدة 429 — ولا يثبت **أسماء حقول السجلّ**.
ربط `struct` بأسماء مخمَّنة بالضبط ما يمنع `AGENTS.md` §6 («لا تخترع أشكال API»).

> **المبدأ: التقاط حقيقي واحد، ثم الربط من المُلاحَظ.**
> كل اسم حقل في §7-3 و §7-4 و §7-5 **مقروء من استجابة حقيقية** — لا مستنتَج ولا مأخوذ من الذاكرة.
> مُرِّر العميل المرجعي Python في `D:\latest_project` كمرجع للسلوك فقط.

### 7-2. قواعد القبول للتسجيلات الحقيقية

| القاعدة | التطبيق |
|---|---|
| **الجسم فقط** | لا تُحفظ الترويسات ولا المفتاح. المفتاح ترويسة طلب فقط — ولا يظهر في أي جسم استجابة. |
| **بلا أسرار** | `x-api-key` لا يُطبع ولا يُكتب ولا يدخل استعلاماً ولا URL. التدقيق في §7-3. |
| **بلا بيانات شخصية** | **هوية المؤلف تُنقَّى**: `id` · `name` · `url` · `avatarUrl` → `example-author-<n>` · `userTierInfo` يبقى. والمقبض نفسه (51 مقبضاً حقيقياً) يُنقَّى أينما ظهر: في `links.sourceUrl` / `links.issuesUrl` (GitHub) · في `patreon.com` · كاسم مدخل أرشيف في `modules[].name`. |
| **المُعرِّفات العامة تبقى** | `modId` · `fileId` · `classId` · معرّفات التصنيفات · `downloadCount` · التواريخ · الهاشات · روابط CDN — بيانات كتالوج عامة، وتبقى كما هي. |
| **التقليم** | المصفوفات الثانوية (`latestFiles` · `latestFilesIndexes` · `screenshots` · `modules`) تُقصّ إلى 1–3 عناصر. كل حقل يحمل معلومة ربط يبقى كاملاً. |
| **حجم صغير** | كل ملف ≤ 16 KB (الحدّ العام في §4-1 هو 100 KB). |
| **التقط لمرة واحدة** | الالتقاط الخام **لا يُحفظ** (يحوي بيانات شخصية) — فلا يمكن إعادة التوليد آلياً. التسجيلات ثابتة. |

### 7-3. الملفات المسجَّلة

المسار: `core/tests/fixtures/json/curseforge/`

| الملف | الطلب | الحجم |
|---|---|---|
| `mods_search.json` | `GET /v1/mods/search?gameId=432&classId=6&pageSize=2&index=0&sortField=2&sortOrder=desc&searchFilter=jei` | 15,655 B |
| `mod_238222.json` | `GET /v1/mods/238222` | 6,713 B |
| `mod_238222_description.json` | `GET /v1/mods/238222/description` | 3,733 B |
| `mod_448233_no_distribution.json` | `GET /v1/mods/448233` | 6,974 B |
| `mod_files_page_index0.json` | `GET /v1/mods/238222/files?pageSize=2&index=0` | 4,872 B |
| `mod_files_page_index2.json` | `GET /v1/mods/238222/files?pageSize=2&index=2` | 4,866 B |
| `file_9277601.json` | `GET /v1/mods/238222/files/9277601` | 2,213 B |
| `mod_files_no_distribution.json` | `GET /v1/mods/448233/files?pageSize=50&index=0` (مُقلَّم إلى ملفَّين) | 3,969 B |
| `fingerprints_request.json` | جسم الطلب لـ `POST /v1/fingerprints` | 43 B |
| `fingerprints_response.json` | `POST /v1/fingerprints` | 6,603 B |

> **نقطة التحقق:** كل ملف مُتحقَّق منه آلياً — Valid JSON · لا المفتاح · لا أي من الـ51 مقبضاً · لا
> `avatarUrl` على CDN خاص · **ولا حقل مفقود** مقارنةً بالالتقاط الخام (مقارنة مجموعة المفاتيح لكل
> سجلّ).

### 7-4. سجلّ المود (mod record) — 30 مفتاحاً

المصدر: `GET /v1/mods/search` (عنصر `data[i]`) و `GET /v1/mods/{id}` (الجسم `data`).
**مجموعة المفاتيح متطابقة تماماً بين النقطتين** (مُتحقَّق منه حقلاً بحقل).

| الحقل | JSON path | النوع | ملاحظة |
|---|---|---|---|
| المعرّف | `data.id` | u32 | `238222` |
| اللعبة | `data.gameId` | u32 | `432` = Minecraft |
| الاسم | `data.name` | String | `"Just Enough Items (JEI)"` |
| المعرّف النصّي | `data.slug` | String | `"jei"` |
| الوصف المختصر | `data.summary` | String | سطر واحد. **لا يوجد حقل `description` في السجلّ** — انظر أدناه. |
| الروابط | `data.links` | Object | `{websiteUrl, wikiUrl, issuesUrl, sourceUrl}` — `websiteUrl` هو رابط المشروع، و`wikiUrl` قد يكون `""` |
| حالة النشر | `data.status` | u32 | |
| عدد التنزيلات | `data.downloadCount` | u64 | `638527706` |
| المفضّلة | `data.favoriteCount` | u32 | |
| مميّز | `data.isFeatured` | bool | |
| التصنيف الأساسي | `data.primaryCategoryId` | u32 | |
| التصنيفات | `data.categories[]` | Array\<Object\> | `{id, gameId, name, slug, url, iconUrl, dateModified, isClass, classId, parentCategoryId}` |
| صنف المود | `data.classId` | u32 | `6` = Minecraft Mods |
| المؤلفون | `data.authors[]` | **Array\<Object\>** | **ليست سلسلة** — `{id, name, url, avatarUrl, userTierInfo?}` · `userTierInfo` = `{tierLevel, downloadsThreshold}` و**اختياري** (حاضر في 62 من 64 سجلّاً) |
| الشعار | `data.logo` | Object | `{id, modId, title, description, thumbnailUrl, url}` — `logo.url` الصورة الكاملة و`logo.thumbnailUrl` المصغّرة |
| الملف الرئيسي | `data.mainFileId` | u32 | |
| آخر الملفات | `data.latestFiles[]` | Array\<File\> | نفس شكل سجلّ الملف (§7-5) |
| فهرس آخر الملفات | `data.latestFilesIndexes[]` | Array\<Object\> | `{gameVersion, fileId, filename, releaseType, gameVersionTypeId, modLoader}` — انظر §7-7 ج |
| فهرس Early Access | `data.latestEarlyAccessFilesIndexes[]` | Array | لوحظ **فارغاً** دائماً في العيّنة |
| التواريخ | `data.dateCreated` · `data.dateModified` · `data.dateReleased` | String | ISO-8601 UTC |
| توزيع مسموح | `data.allowModDistribution` | bool | **يحدّد `downloadUrl`** — انظر §7-7 ب |
| الترتيب بالpopularity | `data.gamePopularityRank` | u32 | |
| متاح | `data.isAvailable` | bool | |
| تعليقات | `data.hasCommentsEnabled` | bool | |
| الرخصة | `data.licenseType` | String | `"MIT License"` |
| إعجاب | `data.thumbsUpCount` | u32 | |
| وسم مميّز | `data.featuredProjectTag` | u32 | |
| اللقطات | `data.screenshots[]` | Array\<Object\> | `{id, modId, title, description, thumbnailUrl, url}` |

> **الوصف الطويل ليس في السجلّ.** له نقطة نهاية منفصلة:
> `GET /v1/mods/{id}/description` → `{ "data": "<html>" }` — **HTML خام** (3.6 KB في العيّنة)،
> لا نصّ عادي. على الواجهة أن تحقّق `data` من نوع `String` لا `Object`.

### 7-5. سجلّ الملف (file record) — 21 مفتاحاً

المصدر: `GET /v1/mods/{id}/files` (عنصر `data[i]`) و `GET /v1/mods/{id}/files/{file_id}` (الجسم `data`).

| الحقل | JSON path | النوع | ملاحظة |
|---|---|---|---|
| المعرّف | `data.id` | u32 | |
| المود | `data.modId` | u32 | |
| اللعبة | `data.gameId` | u32 | `432` |
| متاح | `data.isAvailable` | bool | لوحظ `true` في **725/725** سجلّ |
| الاسم المعروض | `data.displayName` | String | `"15.74.0.228 for Fabric 1.20.1"` — اسم بشري |
| اسم الملف | `data.fileName` | String | `"jei-1.20.1-fabric-15.74.0.228.jar"` — الاسم الحقيقي |
| نوع النشر | `data.releaseType` | u32 | قيم مُرصدة: `1` · `2` · `3`. **دلالة كل قيمة غير مُتحقَّق منها** |
| حالة الملف | `data.fileStatus` | u32 | لوحظ `4` فقط في 725/725 |
| **الهاش** | `data.hashes[]` | **Array\<{value, algo}\>** | `algo=1` ⇒ **SHA-1** (40 خانة hex) · `algo=2` ⇒ **MD5** (32 خانة hex). **لا يوجد SHA-512** |
| التاريخ | `data.fileDate` | String | ISO-8601 UTC |
| الحجم | `data.fileLength` | u64 | البايتات |
| الحجم على القرص | `data.fileSizeOnDisk` | u64 | **اختياري** — غائب في 96 من 396 سجلّاً (الملفات الأقدم) |
| رابط التنزيل | `data.downloadUrl` | **Option\<String\>** | **قد يكون `null`** — §7-7 ب |
| عدد التنزيلات | `data.downloadCount` | u64 | |
| إصدارات اللعبة | `data.gameVersions[]` | Array\<String\> | مختلط: إصدارات + وسوم لودر + `"Client"`/`"Server"` — §7-7 ج |
| إصدارات مرتّبة | `data.sortableGameVersions[]` | Array\<Object\> | `{gameVersionName, gameVersionPadded, gameVersion, gameVersionReleaseDate, gameVersionTypeId}` |
| الاعتماديات | `data.dependencies[]` | Array\<{modId, relationType}\> | `relationType` مُرصد `2` و`3` |
| ملف بديل | `data.alternateFileId` | u32 | `0` = لا بديل |
| حزمة خادم | `data.isServerPack` | bool | |
| البصمة | `data.fileFingerprint` | u32 | بصمة CurseForge الخاصة (ليست هاشاً) — 32 بت |
| الوحدات | `data.modules[]` | Array\<{name, fingerprint}\> | قائمة مداخل الأرشيف، ببصمة u32 لكل مدخل |

> ### ⚠️ الأهمّ: **هل نستطيع التحقّق من هاش التنزيل؟**
> **نعم — بـSHA-1.** الحقل `hashes` مصفوفة كائنات `{value, algo}` وليس حقلاً مسطّحاً:
> `hashes[0] = {value: "<40 hex>", algo: 1}` (SHA-1) و`hashes[1] = {value: "<32 hex>", algo: 2}` (MD5).
> **لا يوجد SHA-512** في الاستجابات المرصودة (725/725 ملفاً يحمل `algo=1` و `algo=2` فقط).
>
> ⇒ `AGENTS.md` §5.5 («verify hash (SHA-1 or SHA-512 per source)») **قابل للتحقّق على CurseForge عبر SHA-1**.
> ⇒ **لا تُقيَّد قيمة `algo` بـ2.** اربطها بـ`hashes[]` كاملاً، واختر `algo == 1`.
> ⇒ البصمة `fileFingerprint` (u32) **ليست بديلاً عن الهاش** — هي بصمة proprietary، ولا يمكن التحقّق منها.

### 7-6. الغلاف والصفحات (envelope & pagination)

| النقطة | الغلاف المُرصد |
|---|---|
| `GET /v1/mods/search` | `{ "data": [ Mod, … ], "pagination": { "index", "pageSize", "resultCount", "totalCount" } }` |
| `GET /v1/mods/{id}/files` | `{ "data": [ File, … ], "pagination": { "index", "pageSize", "resultCount", "totalCount" } }` — **نفس الغلاف تماماً** |
| `GET /v1/mods/{id}` | `{ "data": Mod }` — **بلا `pagination`** |
| `GET /v1/mods/{id}/files/{file_id}` | `{ "data": File }` — **بلا `pagination`** |
| `GET /v1/mods/{id}/description` | `{ "data": "<html>" }` |
| `POST /v1/fingerprints` | `{ "data": { "isCacheBuilt", "exactMatches", "exactFingerprints", "partialMatches", "partialMatchFingerprints", "installedFingerprints", "unmatchedFingerprints" } }` |

`exactMatches[]` = `{ id: <modId>, file: <File>, latestFiles: [ File, … ] }`.

**عينة مرصودة:** `searchFilter=jei` → `totalCount: 436` · `pageSize=2` · `resultCount: 2`.
`GET /v1/mods/238222/files` → `totalCount: 3946`.

### 7-7. نتائج الفحص (probes)

#### أ. `index` = **إزاحة صفحة**، لا فهرس ملف

| الطلب | `pagination` | `data[].id` | `data[].fileName` |
|---|---|---|---|
| `…/files?pageSize=2&index=0` | `{index:0, pageSize:2, resultCount:2, totalCount:3946}` | `9277601` · `9277600` | `jei-1.20.1-fabric-15.74.0.228.jar` · `jei-1.20.1-forge-15.74.0.228.jar` |
| `…/files?pageSize=2&index=2` | `{index:2, pageSize:2, resultCount:2, totalCount:3946}` | `9268986` · `9268985` | `jei-1.20.1-fabric-15.74.0.227.jar` · `jei-1.20.1-forge-15.74.0.227.jar` |

**سجلّان مختلفان ⇒ `index` إزاحة صفحة.** أي أنّ `index=2` مع `pageSize=2` تُرجع الملفَّين
**الثالث والرابع** من نفس الترتيب — لا الملفَّين الأولين.

> ⚠️ **انتبه:** نقطة نهاية الملفات ترجع الملفات **من الأحدث إلى الأقدم**، وبلا `sortField` إطلاقاً.
> فـ`index=0` هو **أحدث** إصدار للمود لا أقدمه. (في `search` يوجد `sortField` — في `files` لا يوجد.)

#### ب. `downloadUrl` — القاعدة **على مستوى المود** لا الملف

| الحالة | `allowModDistribution` | النتيجة |
|---|---|---|
| مود مسموح | `true` | `downloadUrl` سلسلة `https` **غير فارغة** في **663/663** سجلّ (50 موداً) |
| مود ممنوع التوزيع | `false` | `downloadUrl` = **`null`** في **62/62** سجلّ (مود واحد: `448233`) |

- **القيمة الفارغة هي `null` (JSON null) — وليست `""`.** **صفر** سجلّ بـ`""` في 725 سجلّاً.
- **صفر** سجلّ بلا مفتاح `downloadUrl` — المفتاح **موجود دائماً**.
- **كل** الـ`null` الـ62 تخصّ المودّ `448233` وحده — أي أنّ **التفريغ دالة على مستوى المود**، لا عطل
  في ملف بعينه.
- ⇒ اربطها `Option<String>` مع `#[serde(default)]`، و`None` تعني «غير قابل للتنزيل» ⇒ تُتخطّى بصمت.

#### ج. وسوم اللودر داخل `gameVersions[]`

`gameVersions` **مختلط**: إصدارات Minecraft + وسوم لودر + وسمان جانبيان.

| الصنف | القيم المرصودة |
|---|---|
| **أسماء لودر** | `Forge` · `Fabric` · `NeoForge` · `Quilt` |
| **وسوم جانبية** (ليست لودر ولا إصدار) | `Client` · `Server` |
| **إصدارات** | 65 قيمة مُرصدَة في عيّنة 50 موداً: `1.7.10` · `1.8` · `1.8.8` · `1.12` · `1.12.1` · `1.12.2` · `1.13.2` · `1.14` · `1.14.1` … `1.21.11` · `26.1` · `26.1.1` · `26.1.2` · `26.2` · `26.3` |
| **صيغة Snapshot** | `1.14-Snapshot` · `1.15-Snapshot` · `1.16-Snapshot` · `1.18-Snapshot` · `1.19-Snapshot` · `1.20.2-Snapshot` · `1.21.2-Snapshot` · `1.21.5-Snapshot` · `26.1-snapshot` · `26.3-snapshot` |
| **اسم لودر من طرف ثالث** | `Risugami's ModLoader` |

> ⚠️ **أسماء اللودر نصوص حرّة — لا يمكن حصرها بقائمة.**
> ظهر `Risugami's ModLoader` في المودّ `361579` ضمن عيّنة 50 موداً.
> ⇒ صَنِّ اللودر **بمقارنة نصّية** (case-insensitive) على قائمة لودر معروفة، **ولا ترفض** ما لا تعرفه.

> ⚠️ **`gameVersionTypeId` لا يميّز اللودر عن بعضه!**
> `sortableGameVersions[].gameVersionTypeId` = **68441** لكل من `Fabric` و`Forge` و`NeoForge` و`Quilt`،
> و**75208** لكل من `Client` و`Server`. الإصدارات لكلٍّ منها `gameVersionTypeId` خاص.
> ⇒ **مصدر لودر الملف الموثوق هو نصّ `gameVersions`، لا `gameVersionTypeId`.**

> ℹ️ **لكن** `latestFilesIndexes[].modLoader` **رقم صحيح** ومفيد:
> `1 = Forge` · `4 = Fabric` · `5 = Quilt` · `6 = NeoForge` — مُتحقَّق منه بملفّ Lودر واحد لكل قيمة.
> بعض المداخل بلا `modLoader` (JSON `null`).

#### د. ترويسات حدّ المعدّل

**أسماء الترويسات المرصودة على `200`:** `Content-Type` · `Content-Length` / `Transfer-Encoding` ·
`Connection` · `Date` · `Server` · `Via` · `X-Cache` · `X-Amz-Cf-Id` · `X-Amz-Cf-Pop`.

> ⚠️ **لم يظهر أي ترويس `X-Ratelimit-*` في أي استجابة** — لا في `200` ولا في `403` ولا في `404`،
> حتى بعد **470 طلباً متتالياً** لم يتجاوز أيٌّ منها حدّاً ولم يُنتج `429` أصلاً.
> ⇒ عقدة `X-Ratelimit-Reset` التي يعتمدها العميل المرجعي Python **غير مُتحقَّق منها في هذه النشر**.
> نفّذها **دفاعياً**: بحث عن الترويسة **غير حسّاس لحالة الأحرف**، وافتراضي `60` ثانية عند غيابها،
> ولا تفترض وجودها أصلاً.

#### هـ. الاستجابات الخاطئة

| الحالة | المرصود |
|---|---|
| **مفتاح `x-api-key` غائب** | **`403 Forbidden`** — الجسم **فارغ تماماً** (`Content-Length: 0`) |
| **معرّف مود غير موجود** | **`404 Not Found`** — الجسم **فارغ تماماً** (`Content-Length: 0`) |

> ⚠️ **لا يوجد غلاف خطأ JSON.** لا يمكن الاعتماد على `{"error": …}`؛ يجب التفريع على **رمز الحالة**.
> (ترويسة `X-Cache: Error from cloudfront` تظهر في الحالتين.)

### 7-8. خلاصة تنفيذية — ما يجب أن يرتبطه `core`

```rust
// سجلّ المود — 30 مفتاحاً، انظر §7-4
struct CurseForgeMod { id: u32, game_id: u32, name: String, slug: String,
    links: ModLinks, summary: String, status: u32, download_count: u64,
    favorite_count: u32, is_featured: bool, primary_category_id: u32,
    categories: Vec<Category>, class_id: u32, authors: Vec<Author>,  // مصفوفة لا سلسلة
    logo: Option<Logo>, main_file_id: u32, latest_files: Vec<File>,
    latest_files_indexes: Vec<LatestFileIndex>, allow_mod_distribution: bool,
    date_created: String, date_modified: String, date_released: String,
    is_available: bool, license_type: String, /* … */ }

// سجلّ الملف — 21 مفتاحاً، انظر §7-5
struct CurseForgeFile { id: u32, game_id: u32, mod_id: u32, is_available: bool,
    display_name: String, file_name: String, release_type: u32, file_status: u32,
    hashes: Vec<FileHash>,           // {value, algo} — algo=1 هو SHA-1
    file_date: String, file_length: u64,
    file_size_on_disk: Option<u64>,  // اختياري
    download_url: Option<String>,    // null لمودّات allowModDistribution=false
    download_count: u64, game_versions: Vec<String>,
    sortable_game_versions: Vec<SortableGameVersion>, dependencies: Vec<Dependency>,
    alternate_file_id: u32, is_server_pack: bool, file_fingerprint: u32,
    modules: Vec<Module> }
```

**ثلاث قواعد لا تُكسر:**

1. `download_url: Option<String>` — `null` واقع، و`allow_mod_distribution` يفسّره.
2. `hashes` **مصفوفة** `{value, algo}` — استخدم `algo == 1` (SHA-1) للتحقّق (§5.5).
   **لا يوجد SHA-512** — ولا تفترض `algo` لا يتجاوز 2.
3. `authors` **مصفوفة كائنات** — لا سلسلة.

---

## 8. غير مُتحقَّق

| البند |
|---|---|
| الفحص استثنى `.git` — قد تكون فيه fixtures في فروع أخرى |
| لم أقرأ `docs/modrinth-app-gap-analysis.md` (97 KB) — قد يحتوي ردود API مسجّلة |
| ~~**لا توجد استجابات API حقيقية مسجّلة في أي مكان**~~ — **حُلَّ في §7** (تسجيلات CurseForge) |
| أحجام `mtime` الدقة لملفات fixture — غير مقيسة |
| ~~هل نحتاج تسجيلات Modrinth حقيقية لاختبارات `wiremock`~~ — **قرار مفتوح** (تسجيلات CurseForge في §7 تُغطّي النمط نفسه) |
| دلالة `releaseType` = `1` / `2` / `3` — **غير مُتحقَّق** |
| دلالة `fileStatus` = `4` — **غير مُتحقَّق** (لوحظت هذه القيمة وحدها) |
| قيم `gameVersionTypeId` لكل إصدار — مُرصدٌ، **لكنها لا تميّز اللودر** (§7-7 ج) |
| **كل النقاط التي يستخدمها العميل المرجعي Python التُقطت** (`/mods/search` · `/mods/{id}` · `/mods/{id}/description` · `/mods/{id}/files` · `/mods/{id}/files/{file_id}` · `/fingerprints`) — لكن وجود نقاط أخرى في الـAPI v1 (مثل `/categories` · `/minecraft` · `/tags`) **غير مُتحقَّق**: لم تُختبر |
| عقدة `429` وترويسة `X-Ratelimit-Reset` — **غير مُتحقَّق**: لم يُنتج `429` في 470 طلباً، ولم يظهر ترويس `X-Ratelimit-*` إطلاقاً (§7-7 د) |
| `allowModDistribution: false` — لوحظ في مودٍ واحد فقط من 51 موداً؛ **هل كانت النسبة الحقيقية 1/51 أم أكثر في مودّات أخرى؟ غير مُتحقَّق** |
| تسجيلات Modrinth حقيقية — **لم تُلتقط** |