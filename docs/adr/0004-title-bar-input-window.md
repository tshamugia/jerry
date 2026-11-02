# ADR-0004: Title bar: CSS `app-region` და caption ღილაკების input ფანჯარა

- **სტატუსი:** მიღებულია, 02.11.2026. ნამდვილი თაგვით შემოწმება #23-ის დასაწყისშია
- **წყარო:** spike #7 ([#17](https://github.com/tshamugia/jerry/issues/17#issuecomment-6586199502), branch `spike/17-title-bar`); Tech PRD «ფანჯარა და ტაბები»

## კონტექსტი

Tech PRD-ში ცარიელი ზოლი `data-tauri-drag-region` იყო, Snap Layouts-ისთვის კი native hit-test (`HTMAXBUTTON`). `data-tauri-drag-region`-ს JS-ის window API და `core:window:*` permission სჭირდება. Top-level ფანჯრის `WM_NCHITTEST` გვერდის თავზე არასოდეს მოდის, რადგან WebView2-ის ფანჯრები browser process-ს ეკუთვნის.

## გადაწყვეტილება

- ცარიელი ზოლი CSS `app-region: drag`-ია (ტაბებსა და ღილაკებზე `no-drag`). wry რთავს WebView2-ის `IsNonClientRegionSupportEnabled`-ს, ასე გადათრევას, Aero Snap-ს, ორმაგ კლიკს და system menu-ს WebView2 native-ად აკეთებს.
- Caption ღილაკებს (46×40) React ხატავს. თაგვს `jerry-win`-ის უხილავი input ფანჯარა იღებს ღილაკების თავზე (`WS_EX_LAYERED | WS_EX_NOREDIRECTIONBITMAP` child, Windows Terminal-ის მოდელი). ის აბრუნებს `HTMINBUTTON` / `HTMAXBUTTON` / `HTCLOSE`-ს, კლიკზე parent-ს `WM_SYSCOMMAND`-ს უგზავნის, მდგომარეობას კი `Channel`-ით React-ს.
- React Rust-ს ღილაკების მდებარეობას აცნობებს (`title_bar_set_buttons`), Rust ამრავლებს DPI-ზე და ფანჯარას `WM_SIZE` / `WM_DPICHANGED`-ზე თავიდან ათავსებს.
- Mica: `transparent: true` + `DWMWA_SYSTEMBACKDROP_TYPE = DWMSBT_MAINWINDOW` build 22621+-ზე.

## შედეგები

- `core:window:*` permission არ სჭირდება, capabilities ცარიელი რჩება.
- Snap Layouts შესაძლებელია: hit-test კოდები, სტილები, Mica და maximize-ის ზომა probe-ით და unit ტესტებით დადასტურდა.
- ღიაა: აღწევს თუ არა ნამდვილი თაგვი layered ფანჯრამდე (`WindowFromPoint` სხვა პროცესიდან მას ტოვებს). ეს #23-ში პირველ რიგში მოწმდება. Fallback ჩვეულებრივი child ფანჯარაა (`JERRY_PLAIN_INPUT`), რომელიც React-ის ღილაკებს არ უნდა ფარავდეს.
- Caption ღილაკებს tooltip და UI Automation (Narrator) ჯერ არ აქვთ: #23-ის ამოცანაა.

## განხილული ალტერნატივები

- **`data-tauri-drag-region` + JS window API:** permission-ები და JS-ის გამოძახება ყოველ გადათრევაზე; Snap Layouts-ს მაინც ვერ აძლევს.
- **Native caption (`decorations: true`):** ტაბების ზოლი სათაურში ვერ ჩაჯდება და პანელების საზღვარზე ვერ გაიყოფა.
