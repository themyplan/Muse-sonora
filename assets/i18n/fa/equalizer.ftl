equalizer-advanced = پیشرفته
equalizer-simple = ساده
equalizer-sound = صدا
equalizer-profile-default = پیش‌فرض
equalizer-profile-new = پروفایل { $number }
equalizer-profile-pasted = پروفایل چسبانده‌شده
equalizer-profile-name-hint = نام پروفایل
equalizer-save-as = ذخیره به‌عنوان پروفایل
equalizer-rename = تغییر نام پروفایل
equalizer-delete = حذف پروفایل
equalizer-bass = بم
equalizer-treble = زیر
equalizer-tone-reset = بازنشانی به ۰ dB
equalizer-preamp = پری‌امپ
equalizer-preamp-auto = خودکار
equalizer-preamp-manual = دستی
equalizer-decibels = dB
equalizer-hertz = Hz
equalizer-q = Q
equalizer-octaves = اکتاو
equalizer-no-gain = —
equalizer-filters = فیلترها
equalizer-no-filters = هنوز فیلتری نیست. یک فیلتر اضافه کنید، یک پیش‌تنظیم از بالا انتخاب کنید یا یک پروفایل وارد کنید.
equalizer-add-filter = افزودن فیلتر
equalizer-remove-filter = حذف فیلتر
equalizer-filter = فیلتر { $number }
equalizer-filter-kind = نوع فیلتر { $number }
equalizer-kind-peak = پیک
equalizer-kind-low-shelf = شلف پایین
equalizer-kind-high-shelf = شلف بالا
equalizer-kind-low-pass = گذر پایین
equalizer-kind-high-pass = گذر بالا
equalizer-kind-notch = ناچ
equalizer-kind-band-pass = گذر باند
equalizer-kind-all-pass = گذر همه
equalizer-transfer = وارد کردن و صادر کردن
equalizer-import = وارد کردن
equalizer-export = صادر کردن
equalizer-import-file = وارد کردن فایل…
equalizer-export-file = صادر کردن فایل…
equalizer-imported = { $name } وارد شد
equalizer-imported-skipped = { $name } وارد شد، به‌جز { $count ->
        [one] یک خط
       *[other] { $count } خط
    }
equalizer-import-nothing = هیچ تنظیم اکولایزری برای وارد کردن پیدا نشد
equalizer-import-too-many = وارد نشد: { $count ->
        [one] یک فیلتر فعال است
       *[other] { $count } فیلتر فعال است
    }، بیش از ظرفیت یک پروفایل است
equalizer-import-too-large = وارد نشد: متن بیش از ۱ مگابایت است و برای یک پروفایل اکولایزر خیلی بزرگ است
equalizer-filter-limit = یک پروفایل حداکثر { $max } فیلتر دارد.
equalizer-paste-empty = در کلیپ‌بورد متنی نیست
equalizer-copied = پروفایل به‌صورت متن Equalizer APO کپی شد
equalizer-exported = پروفایل صادر شد
equalizer-file-unreadable = فایل خوانده نشد
equalizer-file-unwritable = فایل نوشته نشد
equalizer-value-refused = مقدار قبلی حفظ شد
equalizer-skipped-line = خط { $line }: { $reason }
equalizer-skipped-more = و { $count ->
        [one] یک مورد دیگر
       *[other] { $count } مورد دیگر
    }
equalizer-skip-unknown-type = نوع فیلتر ناشناختهٔ { $name }
equalizer-skip-unreadable = فیلتری نیست که Sonora بتواند بخواند
equalizer-skip-no-frequency = فرکانس ندارد
equalizer-skip-no-gain = گین ندارد
equalizer-skip-no-width = Q یا پهنای باند ندارد
equalizer-skip-expression = مقدار به‌صورت عبارت نوشته شده است
equalizer-skip-no-preamp = پری‌امپ عدد نیست
equalizer-skip-unsupported = { $name } پشتیبانی نمی‌شود
equalizer-rules = قوانین دستگاه
equalizer-rule-other = هر دستگاه دیگری
equalizer-rule-playing = در حال پخش
equalizer-rule-replaces = جایگزین قانون آن می‌شود
equalizer-rule-choose-device = یک دستگاه انتخاب کنید
equalizer-mode = حالت
equalizer-preamp-level = سطح پری‌امپ
equalizer-no-gain-spoken = بدون گین
equalizer-filter-frequency = فرکانس فیلتر { $number }
equalizer-filter-gain = گین فیلتر { $number }
equalizer-filter-width = پهنای فیلتر { $number }
equalizer-graph = پاسخ فرکانسی
equalizer-graph-detail = { $count ->
        [0] بدون فیلتر
        [one] ۱ فیلتر
       *[other] { $count } فیلتر
    }، پری‌امپ روی { $db } dB
equalizer-rule-device = دستگاه
equalizer-rule-choose-sound = یک صدا انتخاب کنید
equalizer-rule-add = افزودن قانون
equalizer-rule-remove = حذف قانون
equalizer-output-port = { $device } · { $port }
equalizer-invalid-frequency = فرکانس باید بین { $low } و { $high } هرتز باشد
equalizer-invalid-gain = گین باید بین { $low } و { $high } دسی‌بل باشد
equalizer-invalid-q = Q باید بین { $low } و { $high } باشد
equalizer-invalid-octaves = پهنای باند باید بین { $low } و { $high } اکتاو باشد
equalizer-invalid-shelf-octaves = یک شلف Q می‌گیرد، نه پهنای باند
equalizer-invalid-preamp = پری‌امپ باید بین { $low } و { $high } دسی‌بل باشد
equalizer-overload = برش به اندازهٔ { $db } دسی‌بل
equalizer-overload-slight = برش کمتر از ۰٫۱ دسی‌بل
equalizer-lower-preamp = کم کردن پری‌امپ
equalizer-level = سطح
equalizer-status-bit-perfect = بیت‌پرفکت
equalizer-status-bit-perfect-detail = بیت‌پرفکت از Sonora
equalizer-status-server = سپس سرور صدا آن را به { $khz } کیلوهرتز تبدیل می‌کند
equalizer-status-altered = بیت‌پرفکت نیست، به دلیل:
equalizer-status-separator = { "، " }
equalizer-status-equalized = EQ
equalizer-status-volume = بلندی صدا
equalizer-status-normalized = نرمال‌شده
equalizer-status-direct = با Soloist پخش می‌شود، نه با Sonora
equalizer-status-rate = { $from } → { $to } کیلوهرتز
equalizer-status-bits = { $from } → { $to } بیت
equalizer-status-float = اعشاری → { $to } بیت
equalizer-status-channels = { $from ->
        [1] مونو
        [2] استریو
       *[other] { $from } کانال
    } → { $to ->
        [1] مونو
        [2] استریو
       *[other] { $to } کانال
    }

# Frequency response graph
response-graph-readout = { $frequency } · { $direction ->
        [up] +{ $gain }
        [down] -{ $gain }
       *[flat] { $gain }
    } دسی‌بل
