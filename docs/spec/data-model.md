# data-model.md — المخطط الحالي، العلاقات، وما يجب تنظيفه

> **المصدر:** `app\storage\schema_contract.py::CREATE_TABLE_STATEMENTS` · `CURRENT_COLUMNS` · `app\storage\migrations\*` · `app\storage\repositories.py` · `app\domain\profile_aggregate.py`
> **⚠️ القسم 6 غير مُلزِم** — ملاحظات نظيفة على النموذج، **لا** مواصفة.

---

## 1. المخطط الكامل — 5 جداول

**النسخ الحرفية للـDDL في `behaviors/storage.md` §1.2.** ملخّص العلاقات هنا:

```
profiles (id PK)
   │ 1
   │
   ├──< profile_mods (profile_id FK, ON DELETE CASCADE, PK=(profile_id, project_id))
   │
   └──  download_history (بلا FK — project_id يُشير إلى Modrith)

favorites      (مستقل تماماً)
settings       (مستقل تماماً)
```

### 1-1. المفاتيح الأجنبية — **واحدة فقط**

```sql
FOREIGN KEY(profile_id) REFERENCES profiles(id) ON DELETE CASCADE
```

مُفعّلة بـ`PRAGMA foreign_keys = ON` على كل اتصال، ومُتحقَّق منها بـ`PRAGMA foreign_key_check` بعد كل ترحيل.

**`project_id` في ثلاث جداول (`favorites` · `profile_mods` · `download_history`) يشير إلى Modrith، لا إلى أي جدول محلي.** **بلا FK، بلا تطبيع.**

### 1-2. الفهارس — **صفر صريح**

فحص `CREATE (UNIQUE )?INDEX|CREATE VIEW|CREATE TRIGGER` عبر المشروع = **صفر نتائج**. كل الفهارس ضمنية:

| الجدول | الفهرس الضمني |
|---|---|
| `favorites` | على `project_id` |
| `profiles` | على `name` (`UNIQUE`) |
| `profile_mods` | على `(profile_id, project_id)` |
| `settings` | على `key` |

### 1-3. `sqlite_sequence`

`AUTOINCREMENT` على `profiles` و`download_history` ⇒ SQLite ينشئ `sqlite_sequence`.

---

## 2. أرقام الأعمدة

| الجدول | أعمدة | مفاتيح | فهارس صريحة | FK |
|---|---|---|---|---|
| `favorites` | **14** | `project_id` PK | 0 | 0 |
| `profiles` | **33** | `id` PK · `name` UNIQUE | 0 | 0 |
| `profile_mods` | **21** | `(profile_id, project_id)` PK | 0 | **1** |
| `download_history` | **14** | `id` PK | 0 | 0 |
| `settings` | **2** | `key` PK | 0 | 0 |
| **المجموع** | **84** | | **0** | **1** |

---

## 3. ثلاثة أنماط متنافسة في مخطط واحد

| # | النمط | التطبيق | العيب |
|---|---|---|---|
| 1 | `INTEGER PRIMARY KEY AUTOINCREMENT` + `TEXT DEFAULT CURRENT_TIMESTAMP` | `favorites.added_at` · `profile_mods.added_at` · `download_history.created_at` | الطابع **بلا `Z`** ولا منطقة زمنية |
| 2 | `TEXT NOT NULL DEFAULT ''` كإشارة «مفقود» | كل أعمدة v2 الـ11 + معظم القديمة | `''` مقابل `NULL` — **دلالتان مختلفتان** |
| 3 | nullable بلا افتراضي | `last_download_at` · `instance_last_validated_at` + 8 أعمدة `download_history` | مختلط مع 1 و2 |

> **ونتيجة أخرى:** `profile_mods.added_at` يُكتب `CURRENT_TIMESTAMP` عبر `DEFAULT`، بينما `profiles.created_at` = `''` نصّاً.

---

## 4. عيوب جدّية

| # | العيب | التفصيل |
|---|---|---|
| 4.1 | **`get_profile` يُسقط 11 عمود v2 بصمت** | `repositories.py:166-169` · وكذلك `list_profiles` (313-316). ثلاثة قرّاء متوازون: `get_profile` · `get_profile_record` · `get_profile_aggregate_metadata` |
| 4.2 | **`ON DELETE CASCADE` صامت** | `delete_profile` (442-444) يمحو كل `profile_mods` بلا سجل تدقيق، بلا `download_history`، بلا سجل عمليات |
| 4.3 | **بلا WAL وبلا busy timeout** | نمط الحجب الافتراضي `delete`، مهلة 0. نافذة ثانية أو عمليتان ⇒ `database is locked` فوراً |
| 4.4 | **`AppRepository.__init__` ينفّذ الترحيلات** | اتصال جديد + قراءة `user_version` **لكل عامل في كل عملية** |
| 4.5 | **`last_download_at` عمود ميت** | **صفر موضع كتابة** عبر `app/` |
| 4.6 | **`updated_at` بلا كاتب واحد** | `update_profile` (381-419) يحدّث 22 عموداً و**لا يمسّ `updated_at`**. فقط `touch_profile_last_used` و`update_profile_aggregate_metadata` |
| 4.7 | **مسارات كتابة متفرّقة** | `create_profile` يكتب 33 · `update_profile` 22 · لا يوجد «تحديث البروفايل» كوحدة |
| 4.8 | **`download_history` بلا فهرس ولا حدّ احتفاظ** | أربعة `LIKE '%…%'` ⇒ فحص كامل مضمون |
| 4.9 | **`favorites` و`profile_mods` نسختان من نفس المستند البعيد** | 13 عموداً مشتركاً **بلا FK ولا مطابقة** |
| 4.10 | **`INSERT OR REPLACE` يهدم `added_at`** | والاست **`added_at` لا يُقرأ في أي مكان** |
| 4.11 | **`_merge_profile_mod_for_upsert` (18 حقلاً)** | حل تعارض heuristic مكتوب يدوياً بسبب 4.9، **ويُصان للأبد**، ويلمس `display_categories` بلا عمود |
| 4.12 | **JSON في TEXT بلا تحقق** | `json.loads(...) or "[]"` ⇒ `JSONDecodeError` يخرج من المستودع |
| 4.13 | **blob إعدادات غير مُنمَّط** | `get_bool_setting` يكرّر truthiness |
| 4.14 | **قائمة الترتيب تكرّر عمود المخطط** | `ALLOWED_DOWNLOAD_HISTORY_SORT_COLUMNS` (18-35) ≡ `schema_contract.CURRENT_COLUMNS["download_history"]` — تنحرف بصمت |
| 4.15 | **`PRIMARY KEY` نصي بلا `NOT NULL`** | `favorites` · `settings` — `NULL` مقبول |
| 4.16 | **نسختان من نفس قوائم الأعمدة** | `schema_contract.CREATE_TABLE_STATEMENTS` و`ADDITIVE_COLUMNS` **ميّتة**؛ ما يعمل فعلاً `v001_contract` |
| 4.17 | **خمس أعمدة = معنى واحد** | `download_dir` · `instance_root` · `minecraft_root` · `mods_dir` · `profile_root` — **وقاعدة الأولوية مُطبَّقة ثلاث مرات** |
| 4.18 | **عالميّان قابلان للتعديل** | `MIGRATIONS` · `CURRENT_SCHEMA_VERSION` — الاختبارات تستبدلهما بـ`mock.patch` |
| 4.19 | **13 عموداً مشتركاً بين `favorites` و`profile_mods`** | نفس مشروع Modrinth يمكن أن يكون مفضّلاً ومثبّتاً بنسختين متعارضتين بلا تسوية |

---

## 5. مخطّط المقترح للتطبيق الجديد

> **غير مُلزِم.** مبني على `foundations.md` §5.

```sql
-- 1. instances  (Naming: ModManager يستخدم "instances" لا "profiles")
CREATE TABLE instances (
    id               INTEGER PRIMARY KEY,
    name             TEXT NOT NULL UNIQUE,
    kind             TEXT NOT NULL,              -- custom | linked_instance | linked_mods_folder | linked_modpack
    ownership        TEXT NOT NULL,              -- app_managed | user_linked | pack_linked | unknown
    game_version     TEXT NOT NULL,
    loader           TEXT NOT NULL,
    loader_version   TEXT NOT NULL DEFAULT '',
    root             TEXT NOT NULL,              -- ✅ نقطة واحدة بدل الخمسة
    created_at       INTEGER NOT NULL,           -- ✅ epoch UTC
    updated_at       INTEGER NOT NULL,
    last_used_at     INTEGER                     -- ✅ NULL = "لم يُستخدم بعد"
);

-- 2. installed_mods  (بدل profile_mods + favorites المكررتين)
CREATE TABLE installed_mods (
    instance_id   INTEGER NOT NULL REFERENCES instances(id) ON DELETE CASCADE,
    project_id    TEXT NOT NULL,
    version_id    TEXT,
    provider      TEXT NOT NULL,
    filename      TEXT NOT NULL,
    sha1          TEXT,
    sha512        TEXT,
    enabled       INTEGER NOT NULL DEFAULT 1,    -- ✅ adject حال التعطيل
    managed       INTEGER NOT NULL DEFAULT 0,    -- ✅ ضد "external / unknown"
    installed_at  INTEGER NOT NULL,
    PRIMARY KEY (instance_id, project_id)
);
CREATE INDEX idx_installed_mods_sha1 ON installed_mods(sha1);   -- ✅ لفحص SHA-1
CREATE INDEX idx_installed_mods_project ON installed_mods(project_id); -- ✅ لفحص التحديثات

-- 3. downloads
CREATE TABLE downloads (
    id              INTEGER PRIMARY KEY,
    instance_id     INTEGER REFERENCES instances(id) ON DELETE SET NULL,
    project_id      TEXT NOT NULL,
    version_id      TEXT,
    filename        TEXT,
    sha1            TEXT,
    destination     TEXT NOT NULL,
    status          TEXT NOT NULL,
    message         TEXT NOT NULL DEFAULT '',
    created_at      INTEGER NOT NULL
);
CREATE INDEX idx_downloads_status_created ON downloads(status, created_at DESC); -- ✅
CREATE INDEX idx_downloads_fingerprint ON downloads(project_id, version_id, sha1); -- ✅ إزالة تكرار

-- 4. api_cache
CREATE TABLE api_cache (
    key        TEXT PRIMARY KEY,
    body       TEXT NOT NULL,
    fetched_at INTEGER NOT NULL
);
CREATE INDEX idx_api_cache_fetched ON api_cache(fetched_at);      -- ✅ إبطال بـ TTL
```

**الإعدادات** ⇒ **ملف JSON في `config_dir`** كما تقول `foundations.md` §5 — **لا جدول**.

---

## 6. ملاحظات التنظيف (غير مُلزِم)

| # | الملاحظة | الأساس |
|---|---|---|
| N-1 | **عمود واحد للمسار** (`root`) + مسارات مشتقّة | 4.17 |
| N-2 | **`kind` و`ownership` في `CHECK` معاً** — مُقيَّدان بـSQLite لا بكود التطبيق | 4.17 · `profile_aggregate._KIND_OWNERSHIP` |
| N-3 | **`ownership` مُنفِّذ** أو محذوف. حقل لا يُقرأ = دَين تقني | 4.4 · `profiles.md` §1.3 |
| N-4 | **طوابع `INTEGER` epoch UTC** بدل ثلاثة ترميزات | 3 |
| N-5 | **`NULL` = مجهول**، لا `''` | 3 |
| N-6 | **`installed_mods` واحدة** بدل `favorites` + `profile_mods` | 4.9 · 4.11 |
| N-7 | **`enabled` عمود** بدل إعادة تسمية `.jar.disabled` | **⚠️ قرار جوهري** — انظر أدناه |
| N-8 | **فهارس** على كل عمود استعلام حقيقي | 4.1 · 4.8 |
| N-9 | **WAL + busy_timeout من اليوم الأول** | 4.3 |
| N-10 | **`created_at`/`updated_at` بكاتب واحد** | 4.6 · 4.7 |
| N-11 | **قائمة الترتيب من `enum`** في Rust ⇒ لا انحراف | 4.14 |
| N-12 | **فصل «افتح القاعدة» عن «رحّل القاعدة»** | 4.4 |
| N-13 | **حذف `last_download_at`** | 4.5 |
| N-14 | **الأصول (assets) المملوكة للتطبيق في `DATA_DIR`** لا في جذر البروفايل | 4.2 · `.modrinth_bulk_downloader_*` داخل البروفايل |

### N-7 — القرار الجوهري: التعطيل

| الخيار | الإيجابي | السلبي |
|---|---|---|
| **(أ) إعادة تسمية `.jar.disabled`** — كما في القديم و`AGENTS.md` §5.12 | متّفق عليه · لا شاشة تغيير · **الملف من خارج التطبيق معطّل أيضاً** | قاعدة البيانات قد تختل عن الواقع إذا عدّل المستخدم الاسم يدوياً |
| **(ب) عمود `enabled`** | المصدر الوحيد للحقيقة = قاعدة البيانات · فهرسة سهلة | **المود لا يتعطّل خارج التطبيق** (Minecraft سيحمّله) |

> **⚠️ توصيتي: (أ)** — التزم بـ`AGENTS.md` §5.12، وهي قاعدة غير قابلة للتفاوض. العمود `enabled` يصبح **مشتقاً** يُحسَب من وجود `.jar.disabled` عند المسح. **أو** احذفه كلياً ولا تدّعِ أنه يعرف.
> **هذا قرار لك.**

---

## 7. غير مُتحقَّق

| البند |
|---|---|
| لم أُشغّل أي استعلام SQL |
| `app\storage\repositories.py` (568 سطراً) — قُرئت الملتقطات مسبقاً لا كاملة |
| Cardinality الفعلية للبيانات في `app.sqlite3` حقيقي |
| هل توجد استعمالات لـ `favorites`/`profile_mods.versions_json` لم تُكتشف |
