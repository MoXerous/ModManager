# WS4 brief — lessons and test coverage

## نطاقك (18 وحدة)
- 10 دروس من docs/spec/lessons.md §1..§10. أهمّها §4.2 (خلط PRESENT/ELIGIBLE أدى إلى حذف بيانات في القديم): تحقّق أن الجديد يفصل المجموعتين فعليًا وباختبار يحمل spec_f131_*.
- 8 وحدات اختبارات: قارن 34 ملف test_*.py قديمًا بـ24 ملف اختبار Rust + 5 TS في الجديد. ركّز على: التنزيل، المزوّدون، السلامة، DB، journal، resolver، mrpack، logging، inventory، i18n.
لكل درس: method=code أو test، وevidence يذكر ملف/اختبارًا حقيقيًا. إذا كان الدرس «تعلّمه القديم بالطريقة الصعبة» وفُعِل في الجديد ⇒ match. إذا عاد الخطأ ⇒ undocumented-deviation severity=blocker.
لا تفترض أن عدد الاختبارات الأكبر يعني تغطية أكبر — اقرأ أسماء الاختبارات وما تختبره.

## الإخراج
ws4.json بصيغة {"dimensionCounts":{"lessons":10,"tests":8},"units":[...]}.
