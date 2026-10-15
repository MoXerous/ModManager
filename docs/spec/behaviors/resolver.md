# behaviors/resolver.md — التبعيات، والتعارضات، والخطة الجافة

> **المرجع:** `app\services\resolver.py` (687 سطراً) · `app\services\batch_install_plan.py` · `app\services\safety_checks.py` · `app\domain\compatibility.py`
> مبدّل Canonical = **Modrinth فقط**.

---

## 1. سياسة مستوى الوحدة

```python
BROWSE_ONLY_PROJECT_TYPES = {"minecraft_java_server"}
STRICT_LOADER_PROJECT_TYPES = {"mod", "modpack"}
```

**`CONTENT_TYPE_LOADER_FALLBACKS`** — قائمة المنصّات لكل نوع محتوى:

| نوع المحتوى | المنصّات بالترتيب |
|---|---|
| `mod` | `fabric, forge, neoforge, quilt` |
| `modpack` | `fabric, forge, neoforge, quilt` |
| `shader` | `iris, optifine, vanilla, minecraft` |
| `resourcepack` | `minecraft` |
| `datapack` | `datapack, minecraft` |
| `plugin` | `paper, spigot, bukkit, folia, purpur, velocity, bungeecord, sponge` |

> **`AGENTS.md` §4.6 يذكر Fabric/Forge/NeoForge/Quilt فقط** — جدول `plugin` و`shader` أعرض منه. **قرار مفتوح لك: هل نحتفظ بكل الأنواع؟**

---

## 2. `candidate_loaders_for_project_type(project_type, selected_loader)`

1. `project_type` افتراضياً `"mod"`؛ `selected` يُخفض ويُقصّ.
2. إن كان `selected` غير فارغ و≠ `"any"`:
   - **نوع صارم** (`mod`/`modpack`) ⇒ **`[selected]` فقط، بلا تدهور عبر اللودرات.**
   - غير صارم: إن كان `selected` ∈ قائمة ذلك النوع ⇒ `[selected] + البقية`، وإلا **تُتجاهل `selected`**.
3. وإلا: قائمة التدهور لنوع المحتوى؛ أو `[selected]` إن كان مضبوطاً و≠ `"any"`؛ أو `["any"]`.

> **القاعدة الحاسمة:** لا تسقط من Fabric إلى Forge لملفات المودات. السبب موثّق في `resolver.py:107-110`: «Falling through from Fabric to Forge/NeoForge (or vice versa) can put an incompatible jar into an active profile.»
> **الاختبار الحارس:** `test_resolver_does_not_fall_back_from_fabric_to_forge_for_mod_files` — يؤكّد `assertNotIn(("get_best_version","project-a","1.21.1","forge",False), client.calls)`.

---

## 3. `version_compatibility_issue(project, version, game_version, loader) -> str`

سلسلة فارغة = متوافق. **الترتيب ثابت:**

| # | الشرط | الرسالة |
|---|---|---|
| 1 | `version is None` | `"No version was selected."` |
| 2 | `version.primary_file() is None` | `"Selected version has no downloadable primary file."` |
| 3 | `version.status != "listed"` | `"Selected version is not listed for download (status: {status})."` |
| 4 | هدف الإصدار (≠ `"any"`، غير فارغ) ∉ `version.game_versions` | `"Selected version does not support Minecraft {t}. Listed game versions: {…\|none listed}."` |
| 5 | هدف اللودر (≠ `"any"`) ولا مرشّح ∈ `version.loaders` | `"Selected version does not support loader/platform {l}. Listed loaders/platforms: {…}."` |

> **مهم:** فحص اللودر يقارن **كل المرشّحين** لا المختار فقط — وهذا ما يمنع التدهور.

---

## 4. `ModResolver.resolve_project(project, game_version, loader, *, allow_alpha=False)`

### 4-1. الخوارزمية

| خطوة | السلوك |
|---|---|
| **0** | بحث في كاش namespace `"resolved"` بمفتاح `_resolve_cache_key(...)` = `(project_id, slug, project_type\|"mod", game_version\|"any", loader\|"any", bool(allow_alpha), preferred_version_id, preferred_version_number, preferred_file_name, preferred_file_sha1.lower(), tuple(preferred_game_versions), tuple(preferred_loaders))` |
| **1** | `hydrate_project()` — إن كان المشروع يملك أياً من `downloads/follows/description/categories/versions` يُعاد **دون لمس**؛ وإلا يُجلب ويُستبدل مع **حفظ كل** `preferred_*`. **أي استثناء ⇒ يُعاد الأصل (stub) بصمت** |
| **2** | `project_type ∈ BROWSE_ONLY_PROJECT_TYPES` ⇒ `version=None`, `status="browse-only"` |
| **3** | `_preferred_version_if_compatible(...)` — **ترتيب التفضيل:** (أ) `preferred_version_id`، (ب) `preferred_file_sha1` عبر `get_versions_from_hashes`، (ج) `preferred_version_number` عبر `get_project_versions(..., filter_compatible=False)` ثم أول إصدار يطابق **و** متوافق. **كل خطوة تبتلع الاستثناءات**؛ فشل الكل ⇒ `None` |
| **4** | **حلقة المرشّحين:** لكل مرشّح لودر (إزالة تكرار بـ`tried`)، `get_best_version(...)`؛ **أول نتيجة صادقة تفوز** ⇒ `status="compatible"`, `reason=""` |
| **5** | تدهور أخير **فقط** لـ `{shader, resourcepack, datapack}` **و** فقط إن لم يجرّب `"any"` |
| **6** | وإلا ⇒ `version=None`, `status="incompatible"`, `reason=_not_found_reason(...)` |

### 4-2. `_not_found_reason` — رسائل نوعية

- `shader`: «…resolved by shader platform (…), **not by Forge/NeoForge directly**.»
- `resourcepack`: رسالة خاصة مشابهة.
- غير ذلك: `"No listed downloadable version for Minecraft {g} + {l}"` (أو بلا `+ {l}` إن كان اللودر فارغاً/`"any"`).

---

## 5. حلّ التبعيات

### 5-1. `_resolve_dependency` — ثلاث طرق

| # | الطريقة | التحقق |
|---|---|---|
| 1 | **`version_id` مثبّت** | يُجلب؛ **إن اختلف `dep_project.project_id` عن `version.project_id` ⇒ `ValueError`** (يبتلعه المُستدعي). متوافق ⇒ سبب `"Using {type} dependency version pinned by Modrinth metadata."` |
| 2 | **`file_name` مثبّت** | `get_project_versions(dep_id, game_version, "any", filter_compatible=False)`؛ أول إصدار `primary_file().filename == file_name` **و** متوافق |
| 3 | وإلا | `resolve_project(dep_project, game_version, loader)` — ⚠ **`allow_alpha` لا يُنقل هنا** |

### 5-2. `_collect_dependency_type` — BFS

- **BFS بـ`queue.pop(0)`** على `resolved_mods` التي `version is not None`.
- `known_project_ids` **يُبذَر** من قائمة الإدخال ويزداد عند كل إضافة ⇒ **كل مشروع يُضاف مرة واحدة فقط، وهذا هو حارس الدورات أيضاً.** دورة تنتهي عند المعرّف المرئي؛ **لا استثناء دورة يُرفع**.
- تخطّي التبعيات بلا `project_id`.
- `loader_overrides.get(dep_project.project_type, loader)` يتجاوز اللودر لكل نوع محتوى.
- فشل الجلب ⇒ العنصر يصبح **stub**: `ContentProject(project_id, slug=id, title="Dependency {id}", description="Could not fetch dependency project metadata.")` مع `status="dependency-error"`, `reason=str(exc)`.
- `recurse_resolved_dependencies=True` **للمطلوبة فقط**. `collect_optional_dependencies` تمرّر `False` ⇒ **الاختيارية عمق واحد فقط**.

### 5-3. مرشّحات مراجعة التبعيات

`collect_dependency_review_candidates(resolved_mods, …)`:
مطلوبة ← اختيارية (محسوبة على `[…resolved_mods, *required]`) ← **سلسلة المطلوبة للأرشيفيات** (`collect_required_dependencies_for_optional_dependencies`) ← دمج بـ`_merge_dependency_review_candidates` حسب `ResolvedContent.key` مع **دمج معرّفات/عناوين الآباء**.

`collect_required_dependencies_for_optional_dependencies` تمشي على حروف `dependency_type == "required"` فقط من مجموعة الاختيارية، وتوسم كل نتيجة بـ`_with_dependency_parent` ⇒ `dependency_requires_selected_parent=True`، والسبب يُلحق بـ`" Required when optional dependency '{title}' is included."`.

> **سلوك المستخدم المؤكَّد:** الاختيارية **غير محدَّدة افتراضياً**، لكن عند تحديد اختيارية تُحدَّد **سلسلتها المطلوبة تلقائياً** (`PHASE16_19_39`).

---

## 6. الكاش — `ResolverRunCache`

| البند | القيمة |
|---|---|
| النوع | في الذاكرة، `threading.RLock`، راية `enabled` |
| النطاقات | `project` · `version` · `hash_versions` · `project_versions` · `best_version` · `resolved` |
| مفتاح `hash_versions` | `(algorithm, tuple(sorted({stripped, lowered, non-empty hashes})))` |
| **الإبطال** | **لا يوجد** |
| العمر | **مقيَّد بالمُستدعي** |

**من نصّ الوثيقة:** «Workers create a fresh instance for each Search/Open/Export/Dependency Review run … This prevents stale Modrinth metadata from being persisted across application sessions».

**التشخيص:**
- `get()` ⇒ `_CACHE_MISS` خاص للوحدة عند التعطيل/الغياب
- `snapshot()` ⇒ `{ns: {hits, misses, entries}}`
- `summary()` ⇒ `"Resolver cache diagnostics: N hit(s), M miss(es), E cached entry/entries [ns h…/m…/e…; …]."`

> **هذا يطبّق قاعدة `lessons.md` §2: كاش داخل تشغيل واحد، لا كاش عام دائم.** في Rust: `RunCache` مملوك لـ`ResolveRun` ومُسقَط بانتهائه.

---

## 7. حالات حدّية **لا يعالجها** الكود القديم صراحةً

| الحالة | السلوك الفعلي |
|---|---|
| **تبعية دورية** | **بلا كشف ولا استثناء.** `known_project_ids` يقتطع الدورة بصمت |
| قائمة إصدارات فارغة / مشروع فارغ | `get_best_version` يعيد `None` ⇒ يسقط إلى `incompatible` |
| **كشف التعارض** | **لا شيء في المبدّل.** حروف `incompatible` لا تُعالَج هنا — تُبلَّغ كتحذيرات فقط (§9 البند 7) |
| `hydrate_project` | **لا يرفع أبداً** |
| `_preferred_version_if_compatible` | **لا يرفع أبداً** |
| `_collect_dependency_type` | **لا يرفع لكل تبعية** |

> `foundations.md` §4.6 تطلب «اكتشاف الدورات وتعارض الإصدارات». **القدرة موجودة في المخطط، غائبة في التنفيذ.** قرار: هل ننفّذها؟ (اقتراحي: نعم، على الأقل **تقرير** بدل صمت.)

---

## 8. الخطة الجافة — `build_batch_install_plan`

> وصف الدالة: «Build a **read-only final batch preview** from already-resolved metadata.»

### 8-1. مدخلات الخطة — `BatchInstallPlanEntry` (frozen)

`source` (`"selected"`|`"dependency"`) · `project_id` · `title` · `project_type` · `version_id` · `version_number` · `filename` · `status` (`"downloadable"`|`"blocked"`) · `reason` · `dependency_type=""`.

| شكل المدخل | `status` | `reason` |
|---|---|---|
| `version is None` | `"blocked"` | `item.reason` أو `compatibility_status` أو `"No compatible downloadable version was resolved."` |
| نسخة بلا `primary_file()` **أو** `file.url` فارغ | `"blocked"` | `"A compatible version was resolved, but it has no primary downloadable file URL."` |
| غير ذلك | `"downloadable"` | `"Ready for download."` |

### 8-2. الملخّص — `BatchInstallPlanSummary` (frozen)
`selected_count` · `dependency_count` · `total_count` · `downloadable_count` · `blocked_count` · `destination` · `target_label` · `game_version` · `loader` · `project_types` · `backup_matching_project_files` · `warnings_count`.

### 8-3. متى يلزم التأكيد
```python
requires_confirmation() = total_count > 1 or dependency_count > 0 or blocked_count > 0 \
                       or warnings_count > 0 or backup_matching_project_files
```

### 8-4. قاعدة إزالة التكرار
المختارة أولاً؛ التبعيات المكرّرة تُسقَط من `entries` لكن **تبقى محسوبة** في `dependency_count`. المفتاح `_project_key = project_id or slug or label`.

### 8-5. ما لا تفعله الخطة **إطلاقاً**

بلا نداء شبكة، بلا تنزيل، بلا كتابة/نقل/حذف، بلا حساب هاش، بلا حل إصدارات (تستهلك `ResolvedContent` محلولة)، بلا حل تبعيات، بلا نسخة احتياطية فعلية (تخزّن فقط رايات منطقيه)، وبلا **حجب** للتعارضات — العناصر المحجوبة **تُسجَّل ولا تُحذف صامتة**.

> **يطابق `AGENTS.md` §5.4: «كل تعديل = خطة ثم تنفيذ، مع دعم dry-run».** النقطة الوحيدة المراد Rivasها: `destination` يُحوَّل إلى نص فقط بـ`Path(...).expanduser()` ثم يُفحص بـ`stat` من باني التحذيرات.

---

## 9. فحوص السلامة — `build_download_safety_warnings`

> «conservative, local preflight warnings before downloading … **never calls the network and never changes user files**».

| # | المُطلِب | الشدّة | **يحجب؟** | الرسالة |
|---|---|---|---|---|
| 1 | نفس `_project_key` بعنوانين مختلفين | `warning` | ❌ | `"The same Modrinth project appears more than once: {a} and {b}."` |
| 2 | `item.version is None` | `warning` | ❌ | `item.reason` أو `"No compatible downloadable version was resolved for the selected Minecraft version/platform."` |
| 3 | `primary_file()` فارغ **أو** `url` فارغ | `warning` | ❌ | `"A compatible version was resolved, but it has no primary downloadable file URL."` |
| 4 | الوجهة/الملف موجود | **`info`** | ❌ | `"A file with this name already exists in the destination. The downloader will skip it if the SHA1 matches, or save a unique filename if it differs."` |
| 5 | مشروعان ⇒ نفس اسم الملف | `warning` | ❌ | `"Two different projects resolve to the same filename: {a} and {b}."` |
| 6 | تبعية مطلوبة خارج المجموعة المحلولة | `warning` | ❌ | `"Required dependency metadata exists but was not included in the resolved download set: …"` (أول 6 ثم `"..."`) |
| 7 | **`incompatible`**Dependency | `warning` | ❌ | `"Modrinth metadata marks one of the selected projects/versions as incompatible: …"` |
| 8 | تبعية **اختيارية** غير مختارة | `info` | ❌ | `"This version lists {n} optional dependenc{y\|ies}. Optional dependencies are shown unchecked … are not auto-selected; if you include one, its required dependencies are included automatically."` |
| 9 | تبعية **`embedded`** | `info` | ❌ | `"This version lists {n} embedded dependenc{y\|ies} in Modrinth metadata."` |

> **لا شيء في هذا الملف يحجب.** ولا توجد شدّة `"error"` **أيناً** في `safety_checks.py`.
> **`embedded` تُحصى ولا تُحلّ ولا تُضاف أبداً.**

### 9-1. التمييز عن `profile_validation`

هناك **سُلّم شدّة ثانٍ** في `app/services/profile_validation.py`: `OK/Warning/Error/Info`، ويستخدم `Error` **فعلاً**. الملفات الثلاث **مفصولة بالكامل** — راجع `profiles.md`.

> **درس (لـ`lessons.md` L-14):** ثلاث سلّمات شدّة متوازية في مشروع واحد (تحميل/تحقق/فحص-أمان + تحقق بروفايل + مُشغِّل عمليات). **وحّدها في Rust** في `IpcError`/تحذيرات مطبَّقة.

---

## 10. الاختبارات

| الاختبار | ما يثبّته |
|---|---|
| `test_resolver_prefers_preferred_version_id_before_latest` | التثبيت يسبق؛ **`assertNotIn(("get_best_version", …), client.calls)`** |
| `test_resolver_can_use_preferred_file_sha1_when_version_id_missing` | تدهور إلى بحث الهاش |
| `test_resolver_can_use_preferred_version_number_when_exact_mismatching` | تدهور إلى رقم الإصدار |
| `test_resolver_falls_back_to_latest_when_pin_is_incompatible` | تقييد غير متوافق ⇒ أحدث |
| **`test_resolver_does_not_fall_back_from_fabric_to_forge_for_mod_files`** | **حارس التعارض الكاذب** |
| `test_resolver_skips_incompatible_pinned_version_for_active_target` | تثبيت 1.20.1 يُتخطّى لهدف 1.21.1 |
| `test_manual_version_mismatch_reason_names_game_version_and_loader` | الرسالة **تسمّي** الإصدار واللودر |
| `test_required_dependency_version_id_is_respected_before_latest` | تثبيت التبع المطلوبة يسبق |
| `test_required_dependency_falls_back_to_latest_when_pin_incompatible` | نفس السياسة للتبعيات |
| `test_dependency_review_candidates_include_optional_unselected_candidates` | الاختيارية تُدرَج مع `dependency_requested_by` |
| `test_dependency_review_candidates_include_required_chain_for_optional_candidates` | **سلسلة 3 أعماق** اختيارية→مطلوبة→مطلوبة متداخلة، مع `dependency_requires_selected_parent` |
| `test_resolver_cache_reuses_duplicate_best_version_result` | نداء واحد + `hits >= 1` + نص التشخيص |
| `test_resolver_cache_reuses_project_versions_for_preferred_number` | الكاش مشترك بين نسختَي مبدّل |
| `test_batch_plan_summarizes_selected_dependencies_and_existing_file_warnings` | الملخّص + تحذير الملفات الموجودة |
| `test_batch_plan_blocks_unresolved_items_before_download_worker` | العناصر المحجوبة تظهر **قبل** بدء العامل |

**تقنية التجسيد الفريدة:** `FakeModrinthClient` يسجّل كل نداء في `self.calls` — والادّعاءات كلها على **شكل** النداء لا على القيمة. **هذه أنقى تقنية اختبار في المشروع كله، وأنصح بنقلها.**

---

## 11. اقتراح للنقل إلى Rust (غير مُلزِم)

| البند | القرار المقترح |
|---|---|
| نطاقات الكاش | `enum CacheNamespace` — يمنع السلاسل الحرّة |
| مفاتيح الكاش | `#[derive(Hash)] struct ResolveKey` بدل `tuple` |
| كشف الدورات | **نفّذ كشفاً صريحاً** وأبلغه — لا تقطع صمتاً |
| `compatibility_status` | `enum CompatibilityStatus` — الكود القديم يحرر **نصوصاً** وهذا مصدر أخطاء صامتة |
| `loader_overrides` | `HashMap<ProjectType, Loader>` مع كاش `OnceLock` للقوائم الساكنة |
| `CONTENT_TYPE_LOADER_FALLBACKS` | `static`/`OnceLock` — هي ثابتة تماماً، وتُعاد بناؤها في كل استدعاء الآن |

## 12. غير مُتحقَّق

| البند |
|---|
| لم أُشغّل أي اختبار |
| `app/services/profile_validation.py` **لم يُقرأ** (سلّم الشدّة الثاني في §9.1 مستنتج من أسماء الاختبارات) |
| `app/ui/workers.py` **لم يُقرأ** — تكامل المبدّل مع الواجهة مستنتج من مواضع الاستدعاء |
| هل جدول `plugin` (8 منصّات) مطلوب فعلاً في التطبيق الجديد |
| سلوك الكاش تحت `tokio` متعدد المهام (الكود القديم خيط-واحد لكل خيط عامل) |