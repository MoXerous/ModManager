# WS2 brief — UI, i18n, safety, performance

## نطاقك (47 وحدة)
- 26 شاشة من docs/spec/screens.md §3 (24 classed + نافذتان بلا صنف: `Target Version Planner` و`Saved mods`) — قارن كل شاشة بالواجهة الحالية في src/app/pages/.
- 4 وحدات i18n: اكتمال en/ar، RTL، مفاتيح خامّة،_plural forms.
- 10 وحدات safety: حواجز المسار، SHA-1/SHA-512، .part ثم rename، الأرشيف enclosed_name، symlinks، الحدود، :disabled، fail-closed redaction، PRESENT مقابل ELIGIBLE، التقليم الجاف.
- 7 وحدات performance (`B-1..B-7`) من docs/spec/performance-budgets.md: throttle التقدّم، التمرير الافتراضي، الإلغاء، الذاكرة، زمن الفحص، حجم الحزمة.

## إلزام المتصفّح
لكل واجهة (26 + i18n): افتح http://localhost:1420/preview-mock.html في تبويب المتصفّح المدمج، ثم snapshot وconsole(level=error) وscreenshot.
- method=browser وevidence يجب أن يحتوي كلمة console ونتيجة الفحص (مثال: "console: 0 errors; raw keys: none").
- احفظ اللقطة في .opencode/analyses/audit/shots/<id>.jpeg واذكر مسارها في evidence.
- أي مفتاح خام أو خطأ console ⇒ undocumented-deviation بseverity high على الأقل.
- إن تعذّر فتح المتصفّح ⇒ verdict=unverifiable واشرح السبب؛ لا تدّعِ النجاح.

## البقية
safety/performance: method=code أو test أو run. اقرأ الكود الفعلي؛ لا تكتفِ باسم الاختبار.

## الإخراج
.opencode/analyses/audit/ws2.json بصيغة {"dimensionCounts":{"ui":26,"i18n":4,"safety":10,"performance":7},"units":[...]}، معرّفات A-0194..A-0240. reviewer="deepseek" للكود، "browser" للواجهات.
