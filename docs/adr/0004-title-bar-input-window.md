# ADR-0004: Title bar: CSS `app-region` და caption ღილაკების input ფანჯარა

- **სტატუსი:** მიღებულია, 02.11.2026. ნამდვილი თაგვით შემოწმდა #23-ში, 05.11.2026
- **წყარო:** spike #7 ([#17](https://github.com/tshamugia/jerry/issues/17#issuecomment-6586199502), branch `spike/17-title-bar`); Tech PRD «ფანჯარა და ტაბები»

## კონტექსტი

Tech PRD-ში ცარიელი ზოლი `data-tauri-drag-region` იყო, Snap Layouts-ისთვის კი native hit-test (`HTMAXBUTTON`). `data-tauri-drag-region`-ს JS-ის window API და `core:window:*` permission სჭირდება. Top-level ფანჯრის `WM_NCHITTEST` გვერდის თავზე არასოდეს მოდის, რადგან WebView2-ის ფანჯრები browser process-ს ეკუთვნის.

## გადაწყვეტილება

- ცარიელი ზოლი CSS `app-region: drag`-ია (ტაბებსა და ღილაკებზე `no-drag`). wry რთავს WebView2-ის `IsNonClientRegionSupportEnabled`-ს, ასე გადათრევას, Aero Snap-ს, ორმაგ კლიკს და system menu-ს WebView2 native-ად აკეთებს.
- Caption ღილაკებს (46×40) React ხატავს. თაგვს `jerry-win`-ის უხილავი input ფანჯარა იღებს ღილაკების თავზე (`WS_EX_LAYERED | WS_EX_NOREDIRECTIONBITMAP` child, Windows Terminal-ის მოდელი). ის აბრუნებს `HTMINBUTTON` / `HTMAXBUTTON` / `HTCLOSE`-ს, კლიკზე parent-ს `WM_SYSCOMMAND`-ს უგზავნის, მდგომარეობას კი `Channel`-ით React-ს. Layered ფანჯარა ნაჩვენებად (და თაგვისთვის ხილულად) მხოლოდ `SetLayeredWindowAttributes(255, LWA_ALPHA)`-ის შემდეგ ითვლება; bitmap-ის გარეშე სრული alpha არაფერს ხატავს (Windows Terminal-იც ასე აკეთებს). Spike-ში ეს გამოძახება არ იყო, ამიტომ `WindowFromPoint` ფანჯარას ტოვებდა.
- Caption ღილაკები ნამდვილი `<button>`-ებია სახელით, Tab-ის რიგის გარეთ: Narrator-ისა და კლავიატურის დაჭერა `title_bar_press`-ით იგივე `WM_SYSCOMMAND`-ს აგზავნის. Input ფანჯრის გარეშე (Windows-ის ჩარჩოიანი ფანჯარა) თაგვი თვითონ მათ აღწევს. Tooltip-ს React აჩენს, როცა თაგვი ღილაკზე 500 ms ჩერდება.
- React Rust-ს ღილაკების მდებარეობას აცნობებს (`title_bar_set_buttons`), Rust ამრავლებს DPI-ზე და ფანჯარას `WM_SIZE` / `WM_DPICHANGED`-ზე თავიდან ათავსებს.
- Mica: `transparent: true` + `DWMWA_SYSTEMBACKDROP_TYPE = DWMSBT_MAINWINDOW` build 22621+-ზე.

## შედეგები

- `core:window:*` permission არ სჭირდება, capabilities ცარიელი რჩება.
- Snap Layouts შესაძლებელია: hit-test კოდები, სტილები, Mica და maximize-ის ზომა probe-ით და unit ტესტებით დადასტურდა.
- ნამდვილი თაგვი layered ფანჯარას აღწევს. #23-ში ხელით შემოწმდა (05.11.2026): hover, tooltip, კლიკი, Snap Layouts flyout, გადათრევა, Aero Snap, ორმაგი კლიკი და system menu. სხვა პროცესიდან `WindowFromPoint` ღილაკებზე input ფანჯარას აბრუნებს, restored-იც და maximized-იც. DPI-ის ცვლილება, სხვა მონიტორი და auto-hide taskbar v0.1-ის ჩეკლისტზეა (#47).
- ჩვეულებრივი child (`WS_EX_LAYERED`-ის გარეშე) მხოლოდ მაშინ ჩნდება, როცა layered child ვერ შეიქმნა (manifest-ის გარეშე test binary).

## განხილული ალტერნატივები

- **`data-tauri-drag-region` + JS window API:** permission-ები და JS-ის გამოძახება ყოველ გადათრევაზე; Snap Layouts-ს მაინც ვერ აძლევს.
- **Native caption (`decorations: true`):** ტაბების ზოლი სათაურში ვერ ჩაჯდება და პანელების საზღვარზე ვერ გაიყოფა.
