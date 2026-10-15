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
```

**كل المولّدات دوال في `core/tests/fixtures/mod.rs` — لا ملفات ثنائية في git.**
(يبقى المستودع خفيفاً، ولا مشاكل ترخيص، ولا اختلافات ثنائية عبر المنصّات.)

---

## 7. غير مُتحقَّق

| البند |
|---|---|
| الفحص استثنى `.git` — قد تكون فيه fixtures في فروع أخرى |
| لم أقرأ `docs/modrinth-app-gap-analysis.md` (97 KB) — قد يحتوي ردود API مسجّلة |
| **لا توجد استجابات API حقيقية مسجّلة في أي مكان** — لم أجد cassettes/HAR |
| أحجام `mtime` الدقة لملفات fixture — غير مقيسة |
| هل نحتاج تسجيلات Modrinth حقيقية لاختبارات `wiremock` (Logging، rate-limit) — **قرار مفتوح** |