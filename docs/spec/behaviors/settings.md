# behaviors/settings.md — الإعدادات المحفوظة ومساراتها

> **المرجع:** `D:\latest_project\` (تطبيق بايثون/PySide6، `master` @ `4f71f18`) · `app\config.py` · `app\storage\repositories.py:595-620` · `app\ui\main_window.py:1069-1310` (`SettingsDialog`)
> ⚠️ **`test_operation_journal.py` لا يحتوي اختبارات إعدادات، ولا يوجد ملف اختبار إعدادات مخصَّص.** التغطية **غير مباشرة** فقط.

---

## 1. التخزين

```sql
CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)
```

⚠️ **`key TEXT PRIMARY KEY` بلا `NOT NULL`** — SQLite لا تستنتجها لـPK نصي، فـ`NULL` مقبول. عيب مخطط حقيقي (`storage.md` §1.3).

**الواجهة البرمجية:**

| الدالة | السطر |
|---|---|
| `AppRepository.get_setting` | 595 |
| `AppRepository.set_setting` | 600 |
| `AppRepository.get_bool_setting` | 607 |
| `AppRepository.set_bool_setting` | 613 |

⚠️ **`get_bool_setting` (607-611) يعيد تنفيذ تحليل truthiness سطرياً `{"1","true","yes","on"}`** — **وهو تكرار حرفي** لـ`config._truthy` (config.py:31-32).
> **عيب تصميمي:** تفسير القيم حقيقي **مكرر في مكانين**. في Rust: `enum BoolSetting` مع `serde` — غير قابل للتكرار.

---

## 2. الإعدادات كاملة — 21 مفتاحاً + ثيم + مسارات

| الثابت | المفتاح | الافتراضي | العنصر | ما يؤثر فيه |
|---|---|---|---|---|
| `SETTING_THEME` | `theme` | `"system"` | combo `{system, light, dark}` | `apply_app_theme:1026` · الألوان · الأنماط · ثيم الغلاف |
| `SETTING_ACCENT_COLOR` | `accent_color` | `"purple"` | combo | ألوان التحديد/التركيز/التقدم |
| `SETTING_SIZE_DISPLAY_UNIT` | `size_display_unit` | `"auto"` | combo `{auto,bytes,kb,mb}` | `_format_file_size:6907`. **القيم المخزَّنة تبقى بايت** |
| `SETTING_PARALLEL_DOWNLOADS` | `parallel_downloads` | **`3`** | spin 1–8 | `max_parallel_downloads` (7443, 14020) |
| `SETTING_DOWNLOAD_RETRIES` | `download_retries` | **`1`** | spin 0–3 | `retry_failed_downloads` ⇒ **محاولتان** |
| `SETTING_SEARCH_LIMIT` | `search_limit` | **`20`** | spin 5–100 | `current_limit` في `start_search:8719` |
| `SETTING_SEARCH_RESOLVE_WORKERS` | `search_resolve_workers` | **`4`** | spin 1–8 | البحث · فتح البروفايل · تصدير mrpack · أثر التبعيات · معاينة الهدف · مراجعة التبعيات |
| `SETTING_PAGINATION_MODE` | `pagination_mode` | `"load_more"` | combo `{Load More, Page Navigation}` | `_apply_pagination_mode_to_ui:8917` |
| `SETTING_REMEMBER_LAST_FILTERS` | `remember_last_filters` | `True` | checkbox | يفعّل استعادة `SETTING_LAST_*` |
| `SETTING_CONFIRM_FAVORITE_DELETE` | `confirm_favorite_delete` | `True` | checkbox | `_confirm_remove_favorite:9588` |
| `SETTING_INCLUDE_REQUIRED_DEPS` | `include_required_dependencies` | `True` | checkbox | هل تدخل التبعياتRequired حوار المراجعة (13955) |
| `SETTING_AUTO_LOAD_METADATA` | `auto_load_metadata` | `True` | checkbox | `load_remote_metadata` عند الإقلاع (8624) |
| `SETTING_AUTO_DISCOVER_ON_STARTUP` | `auto_discover_on_startup` | `False` | checkbox | `_maybe_auto_discover:8685`. **الواجهة تحذّر أنه يكلّف نداء API عند البدء** (1250-1253) |
| `SETTING_SHOW_INSTANCE_TOOLS` | `show_instance_tools` | `True` | checkbox | **واجهة فقط** (4278-4279) |
| `SETTING_PROFILE_INSTANCE_UI_MODE` | `profile_instance_ui_mode` | `False` | checkbox | **واجهة فقط** (9798) |
| `SETTING_MAIN_SPLITTER_SIZES` | `main_splitter_sizes` | — | يحفظ `_save_layout_state:7950` | ارتفاع لوحة السجل؛ **مُتحقَّق بـ`_main_splitter_sizes_fit_current_screen:7938`** |
| `SETTING_RIGHT_SPLITTER_SIZES` | `right_splitter_sizes` | — | `_save_ui_preferences:8016` | عرض لوحة المفتش |
| `SETTING_LAST_PROJECT_TYPE` | `last_project_type` | — | حفظ 8019 / قراءة 7969 | تبويب نوع المحتوى |
| `SETTING_LAST_GAME_VERSION` | `last_game_version` | — | 8020 / 7980 | فلتر الإصدار |
| `SETTING_LAST_LOADER` | `last_loader` | — | 8021 / 7984 | فلتر اللودر |
| `SETTING_LAST_SHADER_PLATFORM` | `last_shader_platform` | `"any"` | 8022 / 7988 | فلتر منصّة الشيدرات |
| `SETTING_LAST_SORT` | `last_sort` | — | 8023 / 7994 | ترتيب النتائج |
| `SETTING_LAST_CLIENT_SIDE` | `last_client_side` | — | 8024 / 8000 | فلتر جهة العميل |
| `SETTING_LAST_SERVER_SIDE` | `last_server_side` | — | 8025 / 8006 | فلتر جهة الخادم |
| `SETTING_LAST_DOWNLOAD_DIR` | `last_download_dir` | — | 8026 / 8012 | نص مجلد التنزيل اليدوي |

> ⚠️ **`SETTING_RIGHT_SPLITTER_SIZES`: لم أجد موضع كتابة — قد يكون ميتاً.** غير مُتحقَّق.

### 2-1. الحماية المدمجة

`_get_int_setting` (6894 / 1283) **يحصر القيمة في المدى عند القراءة** ⇒ القيمة المخزَّنة خارج المدى **لا تصل إلى العامل أبداً**.

> **نمط جيد يستحق النقل:** التحقق عند حدود الاستخدام، لا عند الإدخال فقط.

---

## 3. حوار الإعدادات — `SettingsDialog` (1069)

**4 تبويبات:**

| التبويب | السطر | المحتوى |
|---|---|---|
| `Appearance` | 1090 | الثيم · اللون · وحدة الحجم |
| `Profiles & Discover` | 1094 | وضع الواجهة |
| `Search & Downloads` | 1098 | التوازي · المعاملات · حد البحث · عدد عمال الحل · الترقيم |
| `Storage & License` | 1102 | الترخيص والتفعيل |

---

## 4. إعدادات **خارج** قاعدة البيانات — `app/config.py`

| البند | القيمة |
|---|---|
| `APP_NAME` · `APP_SLUG` · `APP_DIR_NAME` · `APP_AUTHOR` · `APP_VERSION` | 7-11 |
| `USER_AGENT` | 15 |
| `API_BASE_URL` | `"https://api.modrinth.com/v2"` (17) |
| `DEFAULT_GAME_VERSION` | `"1.21.1"` (19) |
| `DEFAULT_LOADER` | `"fabric"` (20) |
| تجاوز بيئي | `MODRINTH_BULK_DOWNLOADER_HOME` (22) · `MODRINTH_BULK_DOWNLOADER_PORTABLE` (23) · ملف `portable.flag` (27) |
| `DB_PATH` | `DATA_DIR / "app.sqlite3"` (102) |

**ترتيب `_resolve_roots` (64-97):**
1. `MODRINTH_BULK_DOWNLOADER_HOME` ⇒ `<override>/{data,config,cache,logs}`
2. `PORTABLE` truthy **أو** `portable.flag` ⇒ `APP_ROOT/user_data/{…}`
3. **توافق قديم:** `~/.modrinth_bulk_downloader/data/app.sqlite3` موجود **و** مسار المنصّة غير موجود ⇒ ترجع كل الجذور إلى `~/.modrinth_bulk_downloader`
4. وإلا: `%APPDATA%` للإعداد · `%LOCALAPPDATA%` لغيره (ويندوز) · `~/Library/…` (macOS) · `XDG_*` (لينكس)

> **في Rust:** `foundations.md` §4.5 يقرر **`directories` ولا كتابة المسارات يدوياً.** هذا يحذف البنود 1-3 كلها دفعة واحدة. **قرار واضح مقترح: احذف بيئة `MODRINTH_BULK_DOWNLOADER_HOME` و`portable.flag`** — لا ن migrations.

---

## 5. ملاحظات ومسائل

| # | الملاحظة | الأثر |
|---|---|---|
| 1 | `settings` جدول **blob غير مُنمَّط بلا مساحة أسماء** | مفتاح واحد يتنازع عليه كل شيء |
| 2 | `get_bool_setting` **يكرّر** `config._truthy` | عند التعديل يُنسى أحدهما |
| 3 | **اختبار إعدادات مخصَّص غير موجود** | سلوك غير مُغطّى |
| 4 | `key TEXT PRIMARY KEY` بلا `NOT NULL` | فجوة مخطط |
| 5 | `SET` يقرأ قيمة من JSON نصية | الإعدادات complex (أحجام splitter) تُخزَّن كنص JSON |
| 6 | `DEFAULT_LOADER = "fabric"` **مثبَّت** | **يناقض `foundations.md` §4.6** الذي يعدّد Fabric/Forge/NeoForge/Quilt |

---

## 6. مقترح للنقل إلى Rust (غير مُلزِم)

| البند | القرار |
|---|---|
| الإعدادات | **ملف JSON في `config_dir`** — كما تقول `foundations.md` §5 صراحةً. **ليس** جدول SQLite |
| النوع | `#[serde(tag = "kind")]` + **تحقّق صارم معSKU** (`deny_unknown_fields`) |
| الأرقام | `u8`/`u16` محصورة بـ`clamp()` — نفس `_get_int_setting` |
| المعاملات | `enum BackoffJitter { None, Full }` — قرار مطلوب (`foundations.md` §4.5 يقول «مع عشوائية») |
| التبويبات | التزم بـ`foundations.md` §9: **الإعدادات** و**التفاصيل** و**المتصفح** و**النسخ** و**التحميلات** و**المود باكات** — بخلاف الـ4 القديمة |
| اللودر الافتراضي | **لا تثبّت** — أول تحميل للودرات |
| بيئة Development | احذف `MODRINTH_BULK_DOWNLOADER_HOME` و`portable.flag` |

## 7. غير مُتحقَّق

| البند |
|---|---|
| لم أُشغّل أي اختبار |
| موضع كتابة `SETTING_RIGHT_SPLITTER_SIZES` |
| القائمة الكاملة للحقول الـ13 المنقّاة (انظر `logging.md`) |
| `packaging/` إعدادات البناء (`build.ps1`) — خارج نطاق |