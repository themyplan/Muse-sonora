members-title = اعضا
members-empty = برای دیدن افرادی که با شما گوش می‌دهند، یک نشست را شروع کنید یا به آن بپیوندید

# login
login-restoring = در حال بررسی جلسه ذخیره‌شده شما…
login-authorizing = در انتظار تأیید در مرورگر شما…
login-signed-in = وارد شده به‌عنوان { $name }
login-failed-title = ورود ناموفق بود
login-problem-region = Spotify از کشوری که در آن هستید جلسه‌ای باز نمی‌کند. از کشور محل سکونت خود وارد شوید یا کشور حساب Spotify خود را تغییر دهید.
login-problem-credentials = جلسه ذخیره‌شده شما برای این سرویس دیگر معتبر نیست. برای ادامه دوباره وارد شوید.
login-problem-password = رمز عبور اشتباه است
login-problem-network = Sonora نتوانست به سرویس موسیقی دسترسی پیدا کند. اتصال اینترنت خود را بررسی کنید و دوباره تلاش کنید.
login-problem-blocked = سرویس موسیقی از شبکهٔ شما اتصال نمی‌پذیرد. اگر VPN استفاده می‌کنید، سرور دیگری را امتحان کنید یا آن را خاموش کنید.
login-problem-cancelled = ورود در مرورگر تأیید نشد یا زمان آن تمام شد. برای تکمیل دوباره شروع کنید.
login-problem-refused = Spotify ورود را رد کرد. کمی صبر کنید و دوباره تلاش کنید.
login-problem-premium = Sonora از طریق Spotify Premium پخش می‌کند و این حساب آن را ندارد. با یک حساب Premium وارد شوید تا ادامه دهید.
login-problem-subscription = اشتراک فعالی روی این Apple ID وجود ندارد.
login-subscription-check = بررسی Apple Music
login-sign-in = ورود با { $provider }
login-connect-cookies = چسباندن دستی کوکی‌ها
login-cookie-open = باز کردن { $provider }
login-cookie-submit = ادامه
login-cookie-hint = سربرگ درخواست Cookie را اینجا بچسبانید
login-cookie-step-1 = music.youtube.com را باز کنید و مطمئن شوید وارد شده‌اید. پنجره ناشناس بهترین گزینه است.
login-cookie-step-2 = F12 را بزنید، زبانه Network را باز کنید و صفحه را دوباره بارگذاری کنید.
login-cookie-step-3 = هر درخواستی با نام «browse» یا «next» را انتخاب کنید.
login-cookie-step-4 = در بخش Headers، Cookie را زیر Request Headers پیدا کنید، روی آن راست‌کلیک کنید و مقدارش را کپی کنید.
login-cookie-step-note = حتماً کل مقدار را بچسبانید، از جمله SAPISID و ‎__Secure-3PAPISID.
login-cookie-header-title = کوکی‌های { $provider } خود را بچسبانید تا ورود کامل شود
login-cookie-apple-step-3 = هر درخواستی به { $site } را انتخاب کنید.
login-cookie-apple-note = حتماً کل مقدار را بچسبانید، از جمله { $cookie }.
login-cookie-named-title = کوکی { $provider } خود را بچسبانید تا ورود کامل شود
login-cookie-named-hint = مقدار کوکی را اینجا بچسبانید
login-cookie-named-step-1 = { $site } را باز کنید و مطمئن شوید وارد شده‌اید.
login-cookie-named-step-2 = F12 را بزنید و زبانه Storage را در Firefox یا زبانه Application را در Chrome باز کنید.
login-cookie-named-step-3 = Cookies را باز کنید، { $site } را انتخاب کنید و کوکی به نام { $cookie } را پیدا کنید.
login-cookie-named-step-4 = روی مقدارش دو بار کلیک کنید و آن را کپی کنید.
login-cookie-named-note = فقط مقدار { $cookie } کافی است. سربرگ کامل Cookie که از زبانه Network کپی شده هم کار می‌کند.
login-guest-title = حالت مهمان
login-guest-use = استفاده از حالت مهمان
login-usage-consent = به ما کمک کنید تخمین بزنیم چند نفر از Sonora استفاده می‌کنند.
login-device-code = این کد را در { $url } وارد کنید
login-server-title = اتصال به سرور Subsonic شما
login-server-detail = Navidrome، Gonic یا هر سرور OpenSubsonic.
login-server-hint = https://music.example.com
login-jellyfin-title = اتصال به Jellyfin
login-jellyfin-detail = با حساب Jellyfin خود وارد شوید.
login-quick-connect = ورود با Quick Connect
login-username-hint = نام کاربری
login-password-hint = رمز عبور
login-server-submit = ورود
login-account-title = انتخاب حساب
login-account-detail = این جلسه به بیش از یک حساب Google وارد شده است. حسابی را که Sonora باید استفاده کند انتخاب کنید.
login-profile-title = انتخاب پروفایل
login-profile-detail = این حساب بیش از یک پروفایل دارد. پروفایلی را که Sonora باید استفاده کند انتخاب کنید.
login-choose-title = ورود به { $provider }
login-choose-detail = نحوه ورود به { $provider } را انتخاب کنید.
login-providers = سرویس‌های موسیقی

# album and playlist pages

# artist page
login-welcome-title = به Sonora خوش آمدید
login-welcome-detail = می‌توانید بعداً در تنظیمات سرویس‌های بیشتری اضافه کنید.
login-or = یا
artist-eyebrow = هنرمند
artist-monthly-listeners =
    { $count ->
        [one] { $value } شنونده ماهانه
       *[other] { $value } شنونده ماهانه
    }
artist-followers = { $count ->
    [one] { $value } دنبال‌کننده
   *[other] { $value } دنبال‌کننده
}
artist-popular = محبوب‌ترین‌ها
artist-popular-eyebrow = آشنایی با این هنرمند
artist-popular-empty = هنوز چیزی از این هنرمند برای پخش نیست
artist-favorite-songs = آهنگ‌های محبوب شما
artist-favorite-songs-count =
    { $count ->
        [one] { $count } آهنگ در علاقه‌مندی‌های شما
       *[other] { $count } آهنگ در علاقه‌مندی‌های شما
    }
artist-releases = انتشارها
artist-filter-all = همه
artist-filter-singles = تک‌آهنگ‌ها
artist-filter-eps = ئی‌پی‌ها
artist-appears-on = حضور در
album-also-like = شاید این‌ها را هم بپسندید
album-tab-artists = هنرمندان
album-label = ℗ { $label }

# user profile page
user-eyebrow = پروفایل
user-followers = { $count ->
    [one] { $value } دنبال‌کننده
   *[other] { $value } دنبال‌کننده
}
user-following = { $count ->
   *[other] { $value } دنبال‌شونده
}
user-playlists = لیست‌های پخش عمومی
user-playlists-empty = هنوز لیست پخش عمومی وجود ندارد

# home page
home-quick-picks = انتخاب‌های سریع
home-quick-picks-empty = چند آهنگ را پسند کنید تا اینجا نمایش داده شوند
home-recently-added = اخیراً اضافه‌شده
home-recently-played = اخیراً پخش‌شده
home-most-played = بیشترین پخش شده
home-favorite-albums = آلبوم‌های محبوب
home-artists = هنرمندان
home-collection-albums = آلبوم‌های مجموعه شما
# powerbar (quick search)

# search page
powerbar-title = جست‌وجوی سریع
powerbar-open = باز کردن
powerbar-song = رفتن به آهنگ
search-placeholder = چه می‌خواهید گوش دهید؟
search-browse = مرور همه
search-submit = جست‌وجو
search-open-link = باز کردن پیوند در Sonora
search-recent = جست‌وجوهای اخیر
search-forget = حذف از تاریخچه
genre-empty = هنوز چیزی اینجا نیست
search-best-match = بهترین نتیجه
search-no-matches = نتیجه‌ای نیست
search-results = نتایج
search-artists = هنرمندان
search-albums-playlists = آلبوم‌ها و لیست‌های پخش
search-tag = { $kind } ·
search-saved =
    { $count ->
        [one] { $count } آهنگ در کتابخانه
       *[other] { $count } آهنگ در کتابخانه
    }

# song page
song-eyebrow = آهنگ
song-view-album = مشاهده آلبوم
song-about = درباره این آهنگ
song-album = آلبوم
song-released = تاریخ انتشار
song-streams = تعداد پخش
song-position = جایگاه
song-label = ناشر
song-popularity = محبوبیت
song-disc-track = دیسک { $disc }، قطعه { $track }
song-track = قطعه { $track }
song-credits = عوامل
song-performed-by = اجرا توسط
song-details = ژانرها و جزئیات
song-genres = ژانرها
song-language = زبان
song-content = محتوا
song-explicit = صریح
song-local = فایل محلی
song-clean = پاک
artist-about = درباره هنرمند
artist-about-fallback = آهنگ‌های محبوب و آثار این هنرمند را بشناسید.
artist-about-open = رفتن به هنرمند
song-copyright = © { $notice }
track-row-name = { $title } از { $artists }

# tags
tags-edit-title = ویرایش برچسب‌ها
tags-sheet-song = آهنگ
tags-sheet-album = آلبوم
tags-sheet-details = جزئیات
tags-title = عنوان
tags-artist = هنرمند
tags-artists = هنرمندان
tags-add-artist = افزودن هنرمند
tags-remove-artist = حذف این هنرمند
tags-track = شماره قطعه
tags-track-total = تعداد قطعه‌های انتشار
tags-disc = شماره دیسک
tags-disc-total = تعداد دیسک‌های انتشار
tags-album = آلبوم
tags-album-artist = هنرمند آلبوم
tags-year = سال
tags-genre = ژانر
tags-composer = آهنگساز
tags-publisher = ناشر
tags-isrc = ISRC
tags-comment = نظر
