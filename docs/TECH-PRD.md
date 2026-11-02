# Jerry — Tech PRD

ტექნიკური სპეციფიკაცია · Oct 31, 2026 · @Tengo

> წყარო: [Claude Doc](https://claude.ai/artifact/M8R2P7e9LGTitUdDakEAzz). ეს ფაილი მისი ასლია (ექსპორტი 02.11.2026).

## მიმოხილვა

Jerry იწყობა Tauri 2-ზე: ბირთვი Rust-ზეა, ინტერფეისი React-სა და TypeScript-ზე, მხოლოდ Windows 10/11-ისთვის. ეს დოკუმენტი აღწერს, როგორ ხორციელდება [Jerry — PRD](PRD.md)-ის მოთხოვნები. PRD ამბობს "რა", ეს დოკუმენტი კი "როგორ": ქცევის ცვლილება ჯერ PRD-ში კეთდება.

### მიღებული გადაწყვეტილებები

| გადაწყვეტილება | მიზეზი |
| --- | --- |
| სტეკი: Tauri 2, Rust, React + TypeScript. საბოლოოა, ალტერნატივები აღარ განიხილება | შეთანხმებულია 31.10.2026 |
| UI ფაილებს არ ეხება: ყველა IO Rust-ის ფონურ thread-ებზე სრულდება | PRD: ინტერფეისი არასოდეს იყინება |
| კოპირება და გადატანა საკუთარი ძრავით (`CopyFileExW`), არა Windows-ის დიალოგით | რიგი, pause/resume, Jerry-ის კონფლიქტების ფანჯარა |
| Recycle Bin-ში წაშლა `IFileOperation`-ით | Explorer-ის იდენტური შედეგი და ზუსტი Undo |
| Drag & drop და clipboard მხოლოდ Windows OLE-ით, შიგნითაც და გარეთაც | სრული თავსებადობა Explorer-თან, Outlook-თან, Telegram-თან; ერთი მექანიზმი |
| სიის მოდელი და დალაგება Rust-შია; ფილტრი, მონიშვნა და რენდერი UI-ში | 100 000 ელემენტი UI-ს არ ტვირთავს |
| ერთი SQLite ფაილი ყველა მუდმივი მონაცემისთვის | პარამეტრები, სესია და ტრანსფერების ჟურნალი ერთ ტრანზაქციულ საცავში |
| ინტერფეისი მხოლოდ ინგლისურადაა, i18n-ის გარეშე; ტექსტი მხოლოდ UI-შია, Rust აბრუნებს შეცდომის კოდს | PRD: ქართული ვერსია არ იქნება; ყველა ტექსტი ერთ ადგილას იწერება |
| ქსელი მხოლოდ განახლების შესამოწმებლად | PRD: სრულად ლოკალური, ტელემეტრიის გარეშე |
| admin-ის უფლებები არასოდეს | PRD: ჩვეულებრივი მომხმარებლის უფლებები |
| Windows 10 22H2+ და Windows 11, მხოლოდ x64 (ARM64 არა) | PRD-ის scope |
| გავრცელება საჯაროა, GitHub repo-ც საჯაროა | release-ები ხელმოწერილია code signing სერტიფიკატით; განახლებები GitHub Releases-დან მოდის |
| Replace-ით ჩანაცვლებული ფაილი Recycle Bin-ში მიდის | PRD: არაფერი იკარგება; Undo მას აბრუნებს |
| DWG preview: მხოლოდ Windows-ის thumbnail | საკუთარი DWG preview საჭირო არ არის |
| რამდენიმე ფანჯარა და ტაბის გატანა v1.0-შია | PRD-ში ეტაპი მითითებული არ იყო |
| ორპანელიანი რეჟიმი ნაგულისხმევია და v0.1-ში იწყობა; ტაბების ზოლი პანელების საზღვარზე ორად იყოფა | PRD: ნაგულისხმევად ორი პანელია |
| განახლება: შეტყობინება ახალ ვერსიაზე და Update ღილაკი | PRD: როგორც სხვა აპებში, მომხმარებლის თანხმობით |

## სტეკი

სტეკი ორ ნაწილად იყოფა: Rust აკეთებს ყველაფერს, რაც ფაილებსა და Windows-ს ეხება, React კი მხოლოდ აჩვენებს და მართავს. ზუსტი ვერსიები scaffold-ისას lockfile-ებში (`Cargo.lock`, `pnpm-lock.yaml`) ფიქსირდება.

### ბირთვი (Rust)

| საჭიროება | ტექნოლოგია |
| --- | --- |
| აპის ჩარჩო: ფანჯრები, IPC, ინსტალერი | Tauri 2 |
| ენა | Rust stable, edition 2024 |
| Win32, COM, Shell, OLE, Cloud Files | `windows` crate (Microsoft-ის ოფიციალური bindings) |
| ასინქრონულობა | tokio (Tauri-ს runtime) |
| პარალელური გამოთვლები: ძებნა, დალაგება | rayon |
| შენახვა | rusqlite (bundled SQLite, WAL) + rusqlite\_migration |
| ZIP | `zip` crate |
| შეცდომები | thiserror (ტიპიზებული კოდები) |
| ლოგები | tracing + tracing-appender |
| IPC ტიპები TS-ში | tauri-specta (ალტერნატივა: ts-rs) |
| Tauri plugin-ები | single-instance, global-shortcut, autostart, updater, process |

### ინტერფეისი (TypeScript)

| საჭიროება | ტექნოლოგია |
| --- | --- |
| რენდერი | WebView2 (Evergreen) |
| UI | React 19 + TypeScript (strict) + Vite |
| სტილი და თემები | Tailwind CSS 4, design tokens CSS ცვლადებად |
| კომპონენტები | Radix UI primitives, shadcn/ui-ის მოდელით (dialog, menu, tooltip, popover) |
| UI-ს აიკონები | lucide-react (ფაილების აიკონები Windows-იდან მოდის) |
| დიდი სიები | TanStack Virtual 3 |
| State | Zustand 5 |
| Command palette, Go to folder | cmdk + uFuzzy |
| შიდა გადალაგება (ტაბები, ფავორიტები, რიგი) | dnd-kit |
| ენები | არ გამოიყენება: ინტერფეისი მხოლოდ ინგლისურადაა |
| Preview | pdf.js (pdfjs-dist), react-markdown + remark-gfm, Shiki |

### ინსტრუმენტები

| საჭიროება | ტექნოლოგია |
| --- | --- |
| პაკეტები | pnpm workspace + Cargo workspace ერთ repo-ში |
| Lint და ფორმატი | Biome (TS), rustfmt + clippy (Rust) |
| ტესტები | cargo-nextest, proptest, criterion, Vitest + Testing Library, WebdriverIO + tauri-driver |
| CI და გამოშვება | GitHub Actions (windows-latest), tauri-action |
| ინსტალერი | NSIS, მომხმარებლის დონეზე |

## არქიტექტურა

Jerry ერთი პროცესია (`jerry.exe`) და WebView2-ის დამხმარე პროცესები. ყველა ფაილური სამუშაო Rust-ის ფონურ thread-ებზე სრულდება, UI კი შედეგებს ნაკადად იღებს.

![Jerry-ის ფენები · UI, IPC, ბირთვი, Windows](assets/tech-architecture.png)

UI ბირთვს ბრძანებებს უგზავნის, შედეგებს კი ნაკადებით, მოვლენებითა და protocol-ებით იღებს. Windows-ს მხოლოდ ბირთვი ეხება.

### Thread-ების მოდელი

| Thread / pool | რას აკეთებს | რატომ აქ |
| --- | --- | --- |
| მთავარი (UI) thread | ფანჯრები, WebView2, clipboard, drag & drop, Windows-ის context menu | OLE-ს STA და ფანჯრის message loop სჭირდება. `OleInitialize` `setup`-ში იძახება: wry მხოლოდ `CoInitializeEx`-ს იძახებს. Modal loop-ები (drag, context menu) `async` ბრძანებიდან ეშვება და Jerry-ის საკუთარ message-only ფანჯარას ეგზავნება (`run_on_main`). Sync ბრძანება WebView2-ის IPC callback-ში სრულდება, Tauri-ს `run_on_main_thread` კი tao-ს callback-ში, სადაც tao Rust → გვერდი მოვლენებს loop-ის დასრულებამდე აჩერებს (spike #4) |
| Shell pool (2–4 STA thread) | აიკონები, thumbnail-ები, `IFileOperation`, Shell თვისებები, OneDrive სტატუსი | Shell extension-ები STA-ს მოითხოვს; timeout იცავს, რომ ერთმა extension-მა არაფერი გაყინოს |
| IO pool | listing, watcher, კოპირება, ZIP | ბლოკირებადი IO UI-ს და IPC-ს არ აჩერებს |
| ტრანსფერის worker-ები | ერთი worker თითო ფიზიკურ დისკზე ან ქსელურ სერვერზე | ერთ დისკზე თანმიმდევრულად, სხვადასხვაზე პარალელურად (PRD) |
| CPU pool (rayon) | ძებნის walker, დალაგება | ყველა ბირთვი |
| tokio runtime | IPC ბრძანებები, მოვლენების გაგზავნა | ასინქრონული ორკესტრაცია |

### Repo-ს სტრუქტურა

```
jerry/
  src/                 React UI
  src-tauri/           Tauri აპი: main, ფანჯრები, IPC ბრძანებები, მოვლენები, protocol-ები
  crates/
    jerry-core/        listing, დალაგება, watcher, ტრანსფერები, Undo, ძებნა,
                       გადარქმევა, ZIP, შენახვა (COM-ის გარეშე, ტესტირებადი)
    jerry-win/         Shell და COM: აიკონები, thumbnail-ები, Recycle Bin, clipboard,
                       drag & drop, context menu, OneDrive, დისკები, რეესტრი, toast
  docs/                ADR-ები, ხელით ტესტირების ჩეკლისტები
```

`jerry-core` არ იცის Tauri-ს და UI-ს შესახებ: მისი ტესტირება შესაძლებელია ჩვეულებრივ დროებით საქაღალდეებზე. `jerry-win` ერთადერთი ადგილია, სადაც COM-ის წესები (apartment, lifetime) დაცულია.

### მონაცემთა ნაკადი: საქაღალდის გახსნა

1. UI იძახებს `listing_open(tab, path)`-ს და იღებს ნაკადს (Channel).
2. ბირთვი IO pool-ზე კითხულობს საქაღალდეს და აგზავნის batch-ებს: პირველს 256 ელემენტზე (100 000-იან საქაღალდეში ~1 ms-ში), შემდეგებს 50 ms-ში ერთხელ, დანარჩენს ბოლოს (spike #2). გაშვებისას სესიის საქაღალდეების listing პროცესის დაწყებისთანავე იწყება, Tauri-მდე: Rust შეტყობინებებს აგროვებს და UI-ს channel-ის მიბმისთანავე უგზავნის (spike #1).
3. ბირთვი ალაგებს და UI-ს უგზავნის ელემენტებს და მათ თანმიმდევრობას.
4. UI ხატავს მხოლოდ ხილულ რიგებს; მათი აიკონები და thumbnail-ები იტვირთება `icon://` და `thumb://` მისამართებით.
5. Watcher აგროვებს ცვლილებებს 100 ms-ის განმავლობაში და იმავე ნაკადში აგზავნის diff-ს.
6. ტაბის დახურვისას ან სხვა საქაღალდეში გადასვლისას UI იძახებს `listing_close`-ს და watcher ჩერდება.

## Rust ბირთვი (jerry-core)

ბირთვი ფლობს ყველა ფაილურ ოპერაციას და მათ უსაფრთხოებას; UI მას აძლევს ბრძანებას და იღებს შედეგს.

### Listing

- `FindFirstFileExW` (`FindExInfoBasic`, `FIND_FIRST_EX_LARGE_FETCH`): სახელი, ზომა, თარიღები და ატრიბუტები ერთი გამოძახებით, ფაილის გახსნის გარეშე.
- ყოველ ელემენტს listing-ის ფარგლებში სტაბილური `entry_id` აქვს. UI ოპერაციებში id-ს გადასცემს, ბირთვი კი ზუსტ `OsString` გზას ინახავს. ასე Windows-ის იშვიათი, არავალიდური UTF-16 სახელებიც უსაფრთხოდ მუშავდება.
- ნელი დისკი (ქსელი, USB): listing გაუქმებადი task-ია, UI ჩატვირთვის ინდიკატორს აჩვენებს და სხვა ტაბები მუშაობს.
- საქაღალდის ზომა მხოლოდ მოთხოვნით ითვლება (Calculate size), ფონზე, ძებნის walker-ით.

### დალაგება

- ბუნებრივი დალაგება Explorer-ის წესით (`StrCmpLogicalW`-ის სემანტიკა): file2 დგას file10-მდე.
- სახელს ერთხელ ექმნება sort key, რომ დალაგების შეცვლა სწრაფი იყოს. სამიზნე: 100 000 ელემენტი 100 ms-ზე ნაკლებში. Spike #2-ში 100 000-ზე Rust-ის sort 2.9 ms-ია, დალაგების შეცვლა კლიკიდან paint-მდე ~30 ms.
- დალაგების შეცვლისას (`listing_sort(id, column, descending)`) UI-ს ეგზავნება მხოლოდ ახალი თანმიმდევრობა (`u32` ინდექსების ბინარული მასივი `tauri::ipc::Response`-ით; 100 000-ზე 400 KB), ელემენტები არა. თარიღით დალაგება ზუსტ `u64` FILETIME-ზე ხდება.
- საქაღალდეები ფაილებზე წინ (პარამეტრი).

### Watcher

- `ReadDirectoryChangesW` ყოველ ღია საქაღალდეზე. ერთი საქაღალდე რამდენიმე ტაბში ერთ watcher-ს იზიარებს.
- ცვლილებები გროვდება 100 ms და იგზავნება diff-ად: დამატებული, წაშლილი, შეცვლილი და ახალი თანმიმდევრობა.
- Buffer-ის გადავსება, ან ქსელური გაზიარება შეტყობინებების გარეშე: სრული ხელახალი listing.

### ტრანსფერების ძრავი

ყოველი კოპირება და გადატანა job-ია რიგში: დათვლა, ადგილის შემოწმება, შესრულება, შეჯამება. ისივე რიგით მუშაობს drag & drop, ZIP და დიდი Undo.

- **რიგები:** ერთი worker თითო ფიზიკურ დისკზე (volume → disk, `IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS`), ქსელისთვის თითო სერვერზე. რიგის თანმიმდევრობა იცვლება.
- **ადგილი:** დაწყებამდე `GetDiskFreeSpaceExW`; თუ არ კმარა, job არ იწყება.
- **კოპირება:** `CopyFileExW` progress callback-ით: პროცენტი, სიჩქარე, pause (callback ელოდება), cancel. დიდ ფაილებზე `COPY_FILE_NO_BUFFERING`.
- **უსაფრთხო ჩაწერა:** ფაილი ჯერ იწერება დროებით სახელზე (`name.ext.jerrypart`, hidden) და დასრულების შემდეგ ერქმევა საბოლოო სახელი. ნახევრად დაკოპირებული ფაილი საბოლოო სახელით არასოდეს ჩნდება.
- **Replace:** ჩანაცვლებული ფაილი Recycle Bin-ში იგზავნება, რომ Undo-ით დაბრუნდეს.
- **იმავე volume-ზე გადატანა:** `MoveFileExW`, მყისიერი, საქაღალდეებიც.
- **სხვა დისკზე გადატანა:** სრული ასლი → ზომის შემოწმება → მხოლოდ ამის შემდეგ წყაროს წაშლა.
- **კონფლიქტი:** აჩერებს მხოლოდ თავის job-ს. "Apply to all" ინახება job-ის პოლიტიკაში. საქაღალდეს კონფლიქტზე: Merge ან Keep both.
- **შეცდომა ფაილზე:** იწერება სიაში, job გრძელდება; ბოლოს Retry მხოლოდ წარუმატებლებზე.
- **დისკი ან ქსელი გაითიშა:** job პაუზდება და ელოდება დაბრუნებას; შემდეგ Resume ან Cancel.
- **საკუთარ თავში კოპირება:** იბლოკება; გზები ედარება `GetFinalPathNameByHandleW`-ით, junction-ების გათვალისწინებით.
- **პროგრესი:** UI-ს ეგზავნება მაქსიმუმ 10-ჯერ წამში.
- **ჟურნალი:** ყოველი job-ის მდგომარეობა SQLite-ში crash recovery-სთვის.

### Undo და Redo

ყოველი დასრულებული ოპერაცია მეხსიერებაში იწერება ინვერსიული მოქმედებების სიად. ინახება ბოლო 50, მხოლოდ სესიის განმავლობაში (PRD).

| ოპერაცია | Undo | რა ინახება |
| --- | --- | --- |
| გადარქმევა, ჯგუფურიც | ძველი სახელები | წყვილები: ახალი → ძველი |
| გადატანა | უკან გადატანა | წყვილები: დანიშნულება → წყარო |
| კოპირება | ასლები Recycle Bin-ში | შექმნილი გზები |
| წაშლა Recycle Bin-ში | აღდგენა ადგილზე | Recycle Bin-ის ელემენტი, რომელსაც `IFileOperation`-ის `PostDeleteItem` აბრუნებს |
| ახალი საქაღალდე ან ფაილი | წაშლა, თუ ცარიელია | გზა |
| ZIP-ის შექმნა ან ამოღება | შედეგი Recycle Bin-ში | შექმნილი გზები |

- Undo-მდე ყოველი ფაილი მოწმდება: file ID, ზომა და შეცვლის დრო უნდა ემთხვეოდეს. შეცვლილი ფაილი გამოიტოვება და მიზეზი ჩანს.
- დიდი Undo (მაგ. 10 GB-ის გადატანის დაბრუნება სხვა დისკიდან) ტრანსფერების რიგში მიდის.

### ძებნა

- საკუთარი პარალელური walker `FindFirstFileExW`-ზე, rayon-ის work-stealing-ით.
- ფილტრები: სახელი (ნაწილობრივი, wildcard, რამდენიმე სიტყვა), ტიპების ჯგუფები, ზომა, თარიღი, ფაილი ან საქაღალდე.
- შედეგები ნაკადად, batch 50 ms-ში ერთხელ, სტატუსით (ნაპოვნია, შემოწმებულია). Stop აჩერებს cancel token-ით.
- Symlink და junction ნაგულისხმევად არ იხსნება; თუ იხსნება, წრეებისგან იცავს volume-ის და file ID-ის სია.
- OneDrive-ზე იკითხება მხოლოდ მეტამონაცემები, შიგთავსი არასოდეს.
- Windows, Program Files და ProgramData ნაგულისხმევად გამორიცხულია.

### გადარქმევა

- სახელის ვალიდაცია ერთი წესებით ორ ადგილას: TS-ში აკრეფისას, Rust-ში საბოლოოდ. იბლოკება აკრძალული სიმბოლოები, დაცული სახელები (CON, NUL, COM1…), ბოლოს წერტილი ან ჰარი.
- Batch rename: ცხრილი "Before → After" TS-ში ითვლება მყისიერად. შესრულება Rust-შია, ორ ფაზად (ჯერ დროებით სახელებზე), რომ a ↔ b გაცვლამაც იმუშაოს. ერთი Undo მთელ ჯგუფზე.
- მხოლოდ რეგისტრის შეცვლა (`file.txt` → `File.txt`) მუშაობს.

### ZIP

- `zip` crate; შექმნა და ამოღება ტრანსფერების რიგში მიდის, პროგრესით და cancel-ით.
- სახელები UTF-8 flag-ით იწერება, რომ ქართული სახელები სწორად გაიხსნას. ძველ არქივებში fallback OEM codepage-ზე.
- ამოღებისას zip-slip დაცვა: `..` და აბსოლუტური გზები იბლოკება.
- ZIP-ის შიგნით ნავიგაცია: ვირტუალური listing, მხოლოდ წასაკითხად.

### დისკები

- სია და ადგილი: `GetLogicalDrives`, `GetDriveTypeW`, `GetDiskFreeSpaceExW`; ქსელური დისკები `WNetGetConnectionW`-ით.
- USB-ის ჩართვა და გამოღება: `WM_DEVICECHANGE`.
- Eject: Shell-ის "Eject" ბრძანება დისკის ელემენტზე; აქტიური ტრანსფერისას ჯერ გაფრთხილება.

## Windows ინტეგრაცია (jerry-win)

Windows-თან ყველა შეხება ერთ crate-შია, რომ COM-ის წესები ერთ ადგილას იყოს. პრინციპი: Jerry იქცევა ისე, როგორც Explorer, და სადაც შესაძლებელია, იგივე Shell API-ს იყენებს.

### აიკონები და thumbnail-ები

- ფაილის აიკონი: `SHGetFileInfoW` + `SHGFI_USEFILEATTRIBUTES` გაფართოებით, დისკზე წვდომის გარეშე. შედეგი system image list-ის ინდექსია და ქეშდება მისი მიხედვით.
- ინდივიდუალური აიკონი მხოლოდ .exe, .lnk, .ico და .url ფაილებს აქვთ.
- Thumbnail: `IShellItemImageFactory::GetImage` Shell pool-ზე, timeout-ით. cloud-only ფაილებზე `SIIGBF_INCACHEONLY`, რომ ჩამოტვირთვა არ დაიწყოს.
- UI მათ იღებს `icon://` და `thumb://` custom protocol-ით. WebView2 თავად ქეშავს და ტვირთავს მხოლოდ ხილულ რიგებს. Thumbnail-ის მისამართში შეცვლის დროა, რომ შეცვლილი ფაილი თავიდან ჩაიტვირთოს.
- საკუთარი disk cache არ არის: Windows-ს უკვე აქვს thumbcache. ზომა ითვლება ეკრანის DPI-ის მიხედვით.

### Clipboard

- Copy და Cut: Explorer-ის Shell data object, CF\_HDROP-ით და Shell IDList Array-ით: `SHCreateShellItemArrayFromIDLists` + `BindToHandler(BHID_DataObject)`, ანუ მშობელი საქაღალდის `GetUIObjectOf`. `SHCreateDataObject` ცარიელ ობიექტს აბრუნებს და არ გამოიყენება (spike #3). `Preferred DropEffect` Explorer-ივით იწერება: Copy = COPY | LINK (5), Cut = MOVE (2). Clipboard-ზე დადებული `IDataObject` მთავარი thread-ის COM ობიექტია და main-thread `thread_local`-ში ინახება. Core-ის `\\?\` პრეფიქსი Shell-ისთვის და CF\_HDROP-ისთვის იშლება: Shell MAX\_PATH-ზე გრძელ ჩვეულებრივ გზას ამუშავებს, `\\?\` გზას კი ვერა. Jerry-ს დახურვისას `OleFlushClipboard`, რომ შიგთავსი დარჩეს: მის გარეშე CF\_HDROP ქრება, flush-ის შემდეგ კი Cut ისევ Cut რჩება (spike #5).
- Paste: CF\_HDROP + `Preferred DropEffect`: MOVE COPY-ის გარეშე = Cut, დანარჩენი Copy. Explorer-იდან ამოჭრილის გადატანის შემდეგ Jerry წერს `Paste Succeeded = MOVE`-ს (`Performed DropEffect`-ის გარეშე) და clipboard-ს თავად ასუფთავებს, თუ `GetClipboardSequenceNumber` არ შეცვლილა: Explorer, როგორც წყარო, `Paste Succeeded`-ზე არ რეაგირებს. Explorer-ში ჩასმული Cut-ის შემდეგ Explorer წერს `Performed DropEffect = MOVE`-ს; Jerry ამაზე არაფერს შლის (ფაილები Explorer-მა უკვე გადაიტანა), მხოლოდ ამოჭრის სტილს აქრობს.
- Virtual files (`FileGroupDescriptorW` + `FileContents`): Outlook-ის attachment ინახება ფაილად.
- Paste as: სურათი (PNG ან CF\_DIBV5) → .png, ტექსტი → .txt.
- `AddClipboardFormatListener` Jerry-ის message-only ფანჯარაზე, მთავარ thread-ზე. ცვლილებისას Jerry ამოწმებს `OleIsCurrentClipboard`-ს (export პირდაპირ იძახება: `windows` crate `S_FALSE`-ს `Ok`-ად აქცევს) და, თუ clipboard მისი აღარ არის, `clipboard:changed`-ით ამოჭრილი ფაილების ნახევრად გამჭვირვალე სტილი ქრება. `WM_CLIPBOARDUPDATE` Jerry-ის საკუთარ ცვლილებაზეც მოდის, ამიტომ ამოჭრის მდგომარეობა `OleSetClipboard`-მდე იწერება.

### Drag & drop

Jerry იყენებს მხოლოდ Windows OLE drag & drop-ს, შიგნითაც და გარეთაც. ამიტომ Jerry-დან გადათრეული ფაილი ყველა პროგრამისთვის Explorer-ის ფაილივია.

- **გადათრევა Jerry-დან:** UI ამჩნევს მაუსის მოძრაობას ზღვარზე მეტად და იძახებს `drag_start(ids)`-ს. ეს `async` ბრძანებაა: tokio pool-იდან drag Jerry-ის საკუთარ ფანჯარას ეგზავნება (`run_on_main`, იხ. «Thread-ების მოდელი»), ასე modal loop მთავარ thread-ზე ეშვება, WebView2-ისა და tao-ს callback-ების გარეთ. იქ ეშვება `SHDoDragDrop` Explorer-ის Shell data object-ით (იხ. Clipboard) და default drop source-ით; drag image და რაოდენობის ბეჯი Windows-ისაა. გვერდი drag-ის შემდეგ `pointerup`-ს არ იღებს, ამიტომ UI drag-ის მდგომარეობას `drag_start`-ის დასრულებისას ასუფთავებს. შემოწმებულია spike #3-ით: Explorer, Desktop, Outlook, Telegram, Chrome და AutoCAD ფაილს ისე იღებს, როგორც Explorer-იდან.
- **მიღება:** საკუთარი `IDropTarget`, მაგრამ არა WebView2-ის ფანჯარაზე. კურსორის ქვეშ ყოველთვის WebView2-ის browser პროცესის ფანჯარაა (`Chrome_WidgetWin_1`, `Chrome_RenderWidgetHostHWND`), და OLE მისგან Jerry-ის ფანჯრებამდე ზემოთ არ ადის: Jerry-ის ფანჯარაზე რეგისტრირებული target გვერდის თავზე არასოდეს გამოიძახება, Tauri-ს ჩაშენებული drop handler-იც არა (spike #4). ამიტომ target გამჭვირვალე owned popup-ზეა (layered, alpha 1/255, არ აქტიურდება), რომელიც გვერდის თავზე მხოლოდ drag-ის დროს ჩნდება. ტაიმერი (50 ms, დაჭერილი ღილაკისას 16 ms) მას აჩენს, როცა Jerry-ს გარეთ დაჭერილი ღილაკით კურსორი Jerry-ზეა ან Jerry-ის საკუთარი drag მიდის. Tauri-ს drop handler გამორთულია (`dragDropEnabled: false`), WebView2-ის `AllowExternalDrop` კი ითიშება, რომ გვერდმა ჩაგდებული ფაილი არ მიიღოს და არ გახსნას. Drag image სამიზნეზე მხოლოდ `IDropTargetHelper`-ით ჩანს.
- **სამიზნის პოვნა:** OLE drag-ის დროს WebView2 მაუსის მოვლენებს ვერ იღებს. ამიტომ Rust აგზავნის `dnd:over {seq, x, y, keys}`-ს (client პიქსელებში, COM გამოძახების გარეთ), UI კი `elementFromPoint`-ით პოულობს სამიზნეს (საქაღალდე, ტაბი, breadcrumb, sidebar) და `dnd_answer`-ით აბრუნებს. მოქმედებას Rust ითვლის. Round trip: მედიანა 4–5 ms (spike #4).
- **`DragOver`** სინქრონულად აბრუნებს მოქმედებას UI-ს ბოლო პასუხით და მიმდინარე ღილაკებით: სამიზნე ერთი კადრით აგვიანებს, Ctrl, Shift და Alt კი მაშინვე მოქმედებს.
- **Drop description** ("Move to Drawings"): `DROPDESCRIPTION` + `IDropTargetHelper`, ტექსტი UI-დან მოდის. სადაც Jerry ჩაგდებას არ იღებს, `DROPIMAGE_NONE` უტექსტოდ, თორემ Windows თავის "+ Copy"-ს წერს.
- **UI-ს ლოგიკა იმავე მოვლენებზე:** spring-loaded folders, ტაბზე 0.7 წმ გაჩერება, კიდესთან auto-scroll.
- **მოქმედება:** იმავე volume → move, სხვა → copy; Ctrl → copy, Shift → move, Alt ან Ctrl+Shift → link. იძულებითი მოქმედება, რომელსაც წყარო არ უშვებს, უარყოფილია; ნაგულისხმევი კი დაშვებულზე გადადის (Outlook და ბრაუზერი მხოლოდ copy-ს უშვებს). მარჯვენა ღილაკით გადათრევა მენიუს აჩვენებს: `Drop` მაშინვე ბრუნდება, მენიუ Jerry-ისაა.
- **ჩაგდების შემდეგ:** CF\_HDROP → ტრანსფერების რიგი. გადატანას Jerry თვითონ აკეთებს (optimized move): `Drop` აბრუნებს NONE-ს ან COPY-ს და წერს `Performed DropEffect = NONE`-ს, რომ წყარომ არაფერი წაშალოს. Virtual files (Outlook, ბრაუზერი): `Drop`-ში მხოლოდ `FileContents`-ის reference-ები აიღება (`AgileReference`), ფაილი worker thread-ზე იწერება `.jerrypart`-ით; IStorage → `.msg`, async წყაროს `StartOperation`/`EndOperation`. სახელი ZIP-ის წესით მოწმდება: `..` და აბსოლუტური გზა იბლოკება.
- **შიდა გადალაგება** (ტაბები, ფავორიტები, რიგი) dnd-kit-ით კეთდება, OLE-ს გარეშე. ტაბის ფანჯრიდან გატანისას კურსორს ფანჯრის გარეთ Rust ადევნებს თვალს.

### Context menu

- Jerry-ის საკუთარი მენიუ React-შია (Radix), ყოველ ბრძანებასთან shortcut-ით.
- "Windows options…": `IShellFolder::GetUIObjectOf` → `IContextMenu` → `QueryContextMenu` → `TrackPopupMenuEx` მთავარ ფანჯარაზე.
- ფანჯრის subclass გადასცემს `WM_INITMENUPOPUP`, `WM_DRAWITEM`, `WM_MEASUREITEM` და `WM_MENUCHAR`-ს `IContextMenu3`-ს. ასე მუშაობს 7-Zip-ის და "Send to"-ს ქვემენიუები.
- ძებნის შედეგებში სხვადასხვა საქაღალდის ფაილების მონიშვნისას Windows-ის მენიუ მიუწვდომელია, განმარტებით.

### გახსნა

- Open: `ShellExecuteExW` ნაგულისხმევი verb-ით.
- Open with: `SHAssocEnumHandlers` (რეკომენდებული პროგრამები) + ბოლოს გამოყენებული SQLite-დან. "სხვა პროგრამა…" ხსნის `SHOpenWithDialog`-ს.
- Open in Terminal: `wt.exe -d <path>`, თუ არ არის, PowerShell.
- Show in Explorer: `SHOpenFolderAndSelectItems`.
- "Open in…" ბრძანებები: პროგრამა + არგუმენტების შაბლონი (`{path}`), გაშვება `CreateProcessW`-ით.
- "ფაილი სხვა პროგრამაშია გახსნილი": პროგრამის სახელს Restart Manager (`RmGetList`) აბრუნებს.

### Recycle Bin

- წაშლა: `IFileOperation` დროშებით `FOFX_RECYCLEONDELETE`, `FOF_ALLOWUNDO`, `FOF_NO_UI`. `PostDeleteItem` აბრუნებს Recycle Bin-ში მოხვედრილ ელემენტს, რომელსაც Undo იყენებს.
- ქსელურ და მოსახსნელ დისკებს Recycle Bin არ აქვთ: Jerry წინასწარ აფრთხილებს და სამუდამო წაშლას ადასტურებით აკეთებს.
- სამუდამო წაშლა (Shift+Del): საკუთარი რეკურსიული წაშლა, რომელიც გრძელ გზებსაც იღებს.
- Recycle Bin-ის ხედი: Shell folder `FOLDERID_RecycleBinFolder`, თვისებებით "ორიგინალი მისამართი" და "წაშლის თარიღი". Restore იყენებს undelete verb-ს, Empty — `SHEmptyRecycleBinW`-ს.

### OneDrive

- სტატუსი ფაილის ატრიბუტებიდან იკითხება, რომლებიც listing-თან ერთად მოდის: `RECALL_ON_DATA_ACCESS` (მხოლოდ cloud-ში), `PINNED` (ყოველთვის მოწყობილობაზე), `UNPINNED`. დამატებითი გამოძახება არ სჭირდება.
- "სინქრონიზდება" სტატუსი: Shell property `System.StorageProviderState`, მხოლოდ ხილული რიგებისთვის, Shell pool-ზე.
- "Always keep on this device" და "Free up space": `CfSetPinState`.
- წესი: Jerry არასოდეს კითხულობს cloud-only ფაილის შიგთავსს მომხმარებლის მოქმედების გარეშე. Preview ასეთ ფაილზე წერს "ფაილი მხოლოდ cloud-შია".

### სისტემური ინტეგრაცია

- გლობალური hotkey: `tauri-plugin-global-shortcut` (`RegisterHotKey`), ნაგულისხმევად Ctrl+Alt+J.
- ერთი ეგზემპლარი: `tauri-plugin-single-instance`; მეორე გაშვების გზა იხსნება ახალ ტაბში ბოლო აქტიურ ფანჯარაში.
- Autostart: `tauri-plugin-autostart` (HKCU Run) `--background` არგუმენტით. ფანჯარა დამალული იქმნება და hotkey-ზე მყისიერად ჩნდება.
- "Open in Jerry": ჩანაწერები `HKCU\Software\Classes`-ში (`Directory\shell`, `Directory\Background\shell`, `Drive\shell`), admin-ის გარეშე. Windows 11-ის ახალ მენიუში გამოსაჩენად v1.0-ში ემატება sparse package (package identity), იმავე code signing სერტიფიკატით; მანამდე ჩანაწერი "Show more options"-ში ჩანს.
- შეტყობინებები: WinRT `ToastNotification` (`windows` crate); დაჭერა (`Activated`) ხსნის დანიშნულების საქაღალდეს. AUMID რეგისტრირდება ინსტალერის shortcut-ით.
- Taskbar-ის პროგრესი: Tauri-ს `set_progress_bar` (`ITaskbarList3`).
- გრძელი გზები: მანიფესტში `longPathAware`, ბირთვი `\\?\` გზებს იყენებს. Shell API-ები 260+ სიმბოლოზე ზოგჯერ ვერ მუშაობს; ასეთ დროს Jerry საკუთარ ოპერაციას იყენებს.

## ინტერფეისი (React)

თითოეულ ფანჯარაში React-ის ერთი აპია, რომელიც მხოლოდ აჩვენებს და მართავს. ფაილებს ის თავად არასოდეს ეხება.

### სტრუქტურა

```
src/
  app/        ფანჯრის ჩარჩო: title bar ტაბებით, layout, ზონების ზომები
  features/   navigation, tabs, file-list, selection, transfers, search,
              preview, sidebar, palette, rename, settings, history
  commands/   ბრძანებების რეესტრი და keymap
  ipc/        tauri-specta-ს გენერირებული bindings და wrapper-ები
  stores/     Zustand store-ები
  strings/    UI-ს ტექსტები და შეცდომების შეტყობინებები (English)
  ui/         საერთო კომპონენტები (Radix, shadcn-ის მოდელით)
```

### State

- Store-ები: ფანჯარა (ნაგულისხმევად ორი პანელი, თითოეულს საკუთარი ტაბები; აქტიური პანელი), ტაბი (ისტორია, ხედი, დალაგება, ფილტრი, scroll, მონიშვნა), ტრანსფერები, პარამეტრები, ფავორიტები.
- მუდმივი მონაცემების წყარო Rust-ია. UI ცვლილებას ბრძანებით აგზავნის, ყველა ფანჯარა კი `settings:changed` მოვლენით ახლდება.
- მონიშვნა: `Set<entry_id>` და anchor. Invert და Select by pattern UI-ში სრულდება.
- სესია ინახება ყოველი ცვლილებისას, 1 წამის debounce-ით, რომ ავარიული დახურვის შემდეგაც აღდგეს.

### ფაილების სია

- TanStack Virtual: Details (რიგები), List (სვეტები), Grid (ბადე). რიგის სიმაღლე ფიქსირებულია სიმჭიდროვის მიხედვით, ამიტომ scroll-ის გამოთვლა მყისიერია.
- სწრაფი ფილტრი (Ctrl+F) UI-შია. სამიზნე: 100 000 სახელის შემოწმება ერთ კადრში (16 ms).
- რიგის კომპონენტი memo-ითაა; მონიშვნის შეცვლა მხოლოდ შეცვლილ რიგებს ხატავს.
- გადარქმევა სიაშივეა (inline input), ვალიდაციით აკრეფისას.

### ფანჯარა და ტაბები

- `decorations: false`. ტაბები და Windows-ის ღილაკები React-შია (ტაბების ზოლი პანელების საზღვარზე ორად იყოფა: თითოეული პანელის ტაბები და მისამართის ზოლი მის ზემოთაა), ცარიელი ადგილი კი `data-tauri-drag-region`.
- Windows 11-ის Snap Layouts maximize ღილაკზე მოითხოვს native hit-test-ს (`HTMAXBUTTON`). ეს spike-ში მოწმდება.
- ყოველი ფანჯარა ცალკე WebView2-ია საერთო browser process-ით.
- ფანჯარა ჩნდება პირველი რენდერის შემდეგ. ფანჯარასა და WebView2-ს თემის ფონი აქვს (`background_color` = `--jerry-mica`): თემა Rust-ში Windows-იდან ფანჯრის შექმნამდე იკითხება. მხოლოდ დამალვა ჩვენებამდე ციმციმს ვერ აცილებს, რადგან `show`-ის მომენტში Chromium-ს კადრი ჯერ არ აქვს: ფონის გარეშე ფანჯარა 8 გაშვებიდან 8-ში თეთრი ჩანდა, ფონით 11-დან 0-ში (spike #1).
- WebView2-ის პარამეტრები: `AreBrowserAcceleratorKeysEnabled = false` (Ctrl+P, Ctrl+F, Ctrl+R და F5 Jerry-ს ეკუთვნის), ნაგულისხმევი context menu და zoom გამორთულია.

### კლავიატურა და ბრძანებები

- ერთი ბრძანებების რეესტრი: id, სათაური, ნაგულისხმევი shortcut, კონტექსტი (სია, გადარქმევა, დიალოგი), handler. მას იყენებს palette, context menu, toolbar, პარამეტრები და F1-ის ცხრილი.
- Shortcut-ები ფიზიკური კლავიშით მოწმდება (`KeyboardEvent.code`). ასე ქართულ განლაგებაზეც Ctrl+C რჩება Ctrl+C.
- Shortcut-ის შეცვლისას კონფლიქტი მოწმდება იმავე კონტექსტში.

### გარეგნობა

- Design tokens CSS ცვლადებად (Tailwind 4 `@theme`): ფერები, accent, სიმჭიდროვე, შრიფტის მასშტაბი. ცვლილება მყისიერია, გადატვირთვის გარეშე.
- System თემა: Tauri-ს `theme` და მისი ცვლილების მოვლენა.
- შრიფტი: Segoe UI Variable. ინტერფეისი ინგლისურია, მაგრამ ფაილის სახელები ხშირად ქართულია. მათი რენდერი spike-ში მოწმდება; საჭიროებისას ემატება ჩაშენებული ქართული შრიფტი.

### ტექსტები

- ინტერფეისი მხოლოდ ინგლისურადაა (PRD). i18n ბიბლიოთეკა არ გამოიყენება; ყველა ტექსტი ერთ ადგილასაა, `src/strings/`-ში.
- Rust ტექსტს არ აბრუნებს, მხოლოდ შეცდომის კოდს და პარამეტრებს. ადამიანური ტექსტი UI-ში იწერება.
- თარიღი DD.MM.YYYY ფორმატით (პარამეტრებში იცვლება), რიცხვები და ზომები `Intl`-ით.
- ქართული ფაილის სახელები სრულად მუშაობს: რენდერი, ბუნებრივი დალაგება, ZIP-ის UTF-8 სახელები.

### Preview

| ტიპი | როგორ |
| --- | --- |
| სურათები | `<img>` asset protocol-ით; SVG მხოლოდ `<img>`-ში, script-ის გარეშე |
| PDF | pdf.js worker-ში; ჯერ პირველი გვერდი, გვერდების რაოდენობა მეტამონაცემებში |
| ტექსტი და კოდი | პირველი 1 MB; გაფერადება Shiki-ით, ენები საჭიროებისას იტვირთება |
| Markdown | react-markdown + remark-gfm, raw HTML-ის გარეშე |
| ვიდეო და აუდიო | `<video>`, `<audio>`; asset protocol Range მოთხოვნებით (გადახვევა) |
| საქაღალდე და ZIP | ბირთვის listing |
| სხვა (DWG, DOCX, XLSX) | Shell thumbnail და Shell-ის თვისებები |

Preview-ს ბიბლიოთეკები ცალკე chunk-ებშია და პირველ preview-ზე იტვირთება, რომ გაშვება არ დამძიმდეს. Asset protocol-ის scope დინამიკურია: Rust უშვებს მხოლოდ იმ ფაილს, რომელიც preview-შია.

## IPC კონტრაქტი

UI და ბირთვი ოთხი გზით უკავშირდებიან ერთმანეთს. ყველა ტიპი Rust-იდან გენერირდება (`src/ipc/bindings.ts`, tauri-specta), და CI ამოწმებს, რომ ფაილი განახლებულია.

| მექანიზმი | როდის | მაგალითები |
| --- | --- | --- |
| ბრძანება (`invoke`) | მოთხოვნა → პასუხი | `rename`, `create_folder`, `delete`, `open`, `settings_set` |
| ნაკადი (Tauri `Channel`) | ეტაპობრივი შედეგი ერთი მომხმარებლისთვის | listing-ის batch-ები და diff-ები, ძებნის შედეგები |
| მოვლენა (`emit`) | ბირთვიდან ყველა ფანჯარაში | `transfer:*`, `drives:changed`, `clipboard:changed`, `settings:changed`, `history:changed`, `dnd:*` |
| Custom protocol | ბინარული მონაცემი `<img>` და `<video>`-სთვის | `icon://`, `thumb://`, asset protocol preview-სთვის |

### ბრძანებები

სახელები სამუშაოა და scaffold-ისას ზუსტდება.

| ჯგუფი | ბრძანებები |
| --- | --- |
| ნავიგაცია | `listing_open`, `listing_close`, `listing_sort`, `resolve_path` (ცვლადები, მოკლე სახელები, ფაილის გზა), `autocomplete_path` |
| ფაილური ოპერაციები | `create_folder`, `create_file`, `rename`, `batch_rename`, `delete`, `duplicate`, `zip_create`, `zip_extract`, `calculate_size` |
| ტრანსფერები | `transfer_enqueue`, `transfer_control` (pause, resume, cancel), `transfer_reorder`, `conflict_resolve`, `transfer_retry_failed` |
| ისტორია | `undo`, `redo`, `history_list`, `history_undo` |
| Clipboard და DnD | `clipboard_copy`, `clipboard_paste`, `paste_as_file`, `drag_start`, `drop_target_update` |
| Shell | `open`, `open_with_list`, `open_with`, `show_native_menu`, `show_in_explorer`, `open_terminal`, `properties` |
| ძებნა | `search_start`, `search_cancel` |
| სისტემა | `drives_list`, `eject`, `onedrive_pin`, `recycle_bin_list`, `recycle_bin_restore` |
| მონაცემები | `settings_get`, `settings_set`, `favorites_*`, `session_save`, `export_settings`, `import_settings` |

### ფორმატი და შეცდომები

- ოპერაციები ფაილებს `entry_id`-ით ან ბირთვის მიერ გაცემული path token-ით მიმართავს. საჩვენებლად გზა ტექსტად მოდის.
- Listing-ის batch ბინარულია (spike #2): ერთი `ArrayBuffer` თითო batch-ზე, `Channel`-ით, little-endian სვეტებად. ჯერ 16-ბაიტიანი header `[kind, count, firstId, nameUnits]`, შემდეგ `f64 size[]`, `f64 modified[]`, `u32 attributes[]`, `u32 nameEnd[]` და სახელები UTF-16-ად; აიკონის გასაღები სვეტად დაემატება. სვეტები გასწორებულია, ამიტომ JS-ში typed array view-ები კოპირების გარეშე იქმნება, სახელებს კი ერთი `TextDecoder("utf-16le")` შიფრავს. ზომა `f64`-ია (ზუსტია 2^53 ბაიტამდე), თარიღი `f64` ms Unix epoch-იდან, ასე BigInt არ სჭირდება. ერთ ნაკადში ოთხი შეტყობინებაა (პირველი `u32` = kind): batch, order, done, error. Rust-ის `Bytes` ტიპი bindings-ში `ArrayBuffer`-ად ჩანს. 100 000 ელემენტის encode + send 4.9 ms-ია, JSON-ით 51.7 ms.
- შეცდომა ერთი ფორმით ბრუნდება: `JerryError { code, path, params }`. კოდები: `AccessDenied`, `NotFound`, `InUse` (პროგრამის სახელით), `NoSpace` (საჭირო და არსებული), `InvalidName`, `DeviceGone`, `NoRecycleBin`, `Cancelled`, `Os` (Windows-ის კოდი).
- UI კოდს ადამიანურ ტექსტად აქცევს და აჩვენებს შემდეგ ნაბიჯს: Retry, Skip, Show in Explorer.
- ტრანსფერის პროგრესი წამში მაქსიმუმ 10-ჯერ იგზავნება; `dnd:over` — კადრში ერთხელ.

## მონაცემები და შენახვა

ყველა მუდმივი მონაცემი ერთ SQLite ფაილშია: `%APPDATA%\Jerry\jerry.db`. ლოგები ინახება `%LOCALAPPDATA%\Jerry\logs`-ში, 14 დღის როტაციით.

| ცხრილი | შიგთავსი | ზღვარი |
| --- | --- | --- |
| `settings` | გასაღები → JSON მნიშვნელობა | — |
| `favorites`, `favorite_groups` | გზა, სახელი, ჯგუფი, რიგი | — |
| `recent_folders` | გზა, ბოლო გახსნა, გახსნების რაოდენობა | 500 |
| `folder_views` | გზა, ხედი, დალაგება, სვეტები | 5 000, ყველაზე ძველი იშლება |
| `session` | ფანჯრები, ტაბები, პანელები, ტაბების ისტორია (JSON) | ერთი ჩანაწერი |
| `search_history` | მოთხოვნა და ფილტრები | 20 |
| `open_with_recent` | გაფართოება, პროგრამა, დრო | — |
| `custom_commands` | "Open in…" ბრძანებები | — |
| `keybindings` | ბრძანება → shortcut, მხოლოდ შეცვლილი | — |
| `transfer_journal` | job, წყაროები, დანიშნულება, სტატუსი, დროებითი ფაილები | დასრულებული იშლება 7 დღეში |

- WAL რეჟიმი. მიგრაციები `rusqlite_migration`-ით, სქემის ვერსიით. მიგრაციამდე იქმნება ფაილის ასლი.
- Undo-ს ისტორია მხოლოდ მეხსიერებაშია და Jerry-ს დახურვისას იშლება (PRD).
- აიკონები მეხსიერებაში ქეშდება, thumbnail-ები Windows-ის thumbcache-ში. Jerry საკუთარ cache-ს დისკზე არ წერს.
- Export/Import: ერთი JSON ფაილი (settings, favorites, keybindings, custom\_commands) სქემის ვერსიით. იმპორტი ანაცვლებს არსებულს და მანამდე ავტომატურად ინახავს ასლს.

## წარმადობა

PRD-ის ყოველ სიჩქარის მიზანს აქვს თავისი ტექნიკა და გაზომვის წესი. საზომი მანქანა შენი სამუშაო კომპიუტერია; ქსელურ დისკზე გაზომვა ყოველ ეტაპზე ერთხელ ტარდება.

| PRD-ის მიზანი | ტექნიკა | როგორ ვზომავთ |
| --- | --- | --- |
| გაშვება < 1 წმ | სესია იკითხება ფანჯრამდე; ორივე პანელის listing იწყება პროცესის დაწყებისთანავე, WebView2-ის ჩატვირთვის პარალელურად; ფანჯრისა და WebView2-ის ფონი თემიდან; საწყისი JS ≤ ~300 KB (+100 KB ≈ +3.6 ms), palette, rename, settings, preview და dnd-kit lazy | `CreateProcess` → პირველი რიგის Element Timing `renderTime` (`startup_probe`), NVIDIA-ს მდგომარეობისა და კვების ჩაწერით; ცივი და თბილი გაშვება |
| 1 000 ელემენტი < 0.2 წმ | `FindFirstFileExW` large fetch, პირველი batch მაშინვე | `listing_open` → paint |
| 10 000+ სიაში scroll შეფერხების გარეშე | ვირტუალიზაცია, ფიქსირებული სიმაღლე, აიკონები ქეშიდან, thumbnail-ები lazy | DevTools Performance, 60 fps |
| 100 000+ ეტაპობრივად | batch-ები, დალაგება Rust-ში, თანმიმდევრობა ბინარულად | პირველი რიგები < 0.3 წმ, დალაგება < 100 ms |
| ძებნა 50 000 ფაილში, პირველი შედეგები < 1 წმ | პარალელური walker, შედეგები ნაკადად | benchmark ფიქსირებულ საქაღალდეზე |
| ინტერფეისი არასოდეს იყინება | IO არასოდეს UI-ში; პროგრესის მოვლენები შეზღუდულია | dev რეჟიმში 50 ms-ზე გრძელი task-ები ლოგში იწერება |

- ბირთვის benchmark-ები (criterion): listing, დალაგება, ძებნა სინთეზურ საქაღალდეებზე (1 000, 10 000, 100 000 ფაილი). 20%-ზე მეტი გაუარესება CI-ში გაფრთხილებას იძლევა.
- მეხსიერების ბიუჯეტი: ერთი ფანჯარა 10 ტაბით < 300 MB WebView2-ის ჩათვლით. Spike #1-ში (private working set): ერთი ტაბი 110 MB, 10 × 1 000 — 119 MB, 8 × 1 000 + 10 000 + 100 000 — 149 MB; აქედან `jerry.exe` 5–14 MB-ია.

## საიმედოობა და უსაფრთხოება

Jerry-ს შეცდომამ ფაილი არასოდეს უნდა დაკარგოს. ამიტომ ყოველი დესტრუქციული ოპერაცია ან შექცევადია, ან წინასწარ დადასტურებული.

### მონაცემების დაცვის წესები

1. წყარო იშლება მხოლოდ დანიშნულებაზე სრული და შემოწმებული ასლის შემდეგ.
2. ფაილი იწერება `.jerrypart` სახელზე და საბოლოო სახელს მხოლოდ დასრულების შემდეგ იღებს.
3. Replace-ით ჩანაცვლებული ფაილი Recycle Bin-ში მიდის.
4. Undo ეხება მხოლოდ უცვლელ ფაილს (file ID, ზომა, შეცვლის დრო).
5. ოპერაცია, რომელსაც Recycle Bin ვერ დაიცავს (ქსელი, USB, გრძელი გზა), წინასწარი გაფრთხილებით სრულდება.
6. სისტემური საქაღალდეების (Windows, Program Files, Users-ის ფესვი) წაშლა და გადატანა დამატებით დადასტურებას მოითხოვს.
7. Jerry არასოდეს შლის ფაილს მომხმარებლის მოქმედების გარეშე. ერთადერთი გამონაკლისია მისი საკუთარი `.jerrypart` ფაილები, ჟურნალის მიხედვით.

### Crash recovery

- Job ჟურნალში იწერება დაწყებამდე, მისი მდგომარეობა კი ფაილების ჯგუფებად ახლდება.
- გაშვებისას დაუსრულებელი job-ები ჩანს: რა დასრულდა და რა არა. მომხმარებელი ირჩევს Resume-ს ან Discard-ს; დარჩენილი `.jerrypart` ფაილები იშლება.
- ფონური worker-ის panic აპს არ აჩერებს: იჭერს `catch_unwind`, იწერება ლოგში და job შეცდომით სრულდება.
- ჩამოკიდებული Shell extension: Shell pool-ში timeout-ის შემდეგ worker ახალით იცვლება.

### უსაფრთხოება

- Tauri capabilities: WebView-ს მხოლოდ Jerry-ის ბრძანებები აქვს. `fs`, `shell` და `http` plugin-ები არ გამოიყენება. Jerry-ის მოვლენების მოსასმენად გვერდს `core:event:allow-listen` და `core:event:allow-unlisten` აქვს.
- CSP: მხოლოდ `self`, `icon:`, `thumb:` და asset protocol. `eval`, inline script და გარე მისამართები აკრძალულია.
- DevTools მხოლოდ dev build-შია.
- ფაილის შიგთავსი script-ს ვერ გაუშვებს: SVG მხოლოდ `<img>`-ით, Markdown raw HTML-ის გარეშე, pdf.js `isEvalSupported: false`-ით.
- ქსელი მხოლოდ განახლების შემოწმებისთვისაა (გამორთვადი). ტელემეტრია არ არის.
- უფლებები: მანიფესტში `asInvoker`; Jerry admin-ს არასოდეს ითხოვს.

## ტესტირება და ხარისხი

ტესტების ძირითადი წონა ბირთვზეა, რადგან ფაილების უსაფრთხოება იქ წყდება. OLE-ის სცენარები სხვა პროგრამებთან ხელით მოწმდება, ჩეკლისტით.

| დონე | რას ფარავს | ინსტრუმენტი |
| --- | --- | --- |
| Rust unit | სახელის ვალიდაცია, ბუნებრივი დალაგება, wildcard, batch rename-ის გეგმა, Undo-ს ინვერსიები | cargo-nextest |
| Rust ინტეგრაცია | ტრანსფერები დროებით საქაღალდეებზე: კონფლიქტები, cancel, pause, დაკავებული ფაილი, გრძელი გზები, ქართული სახელები, zip-slip | cargo-nextest + tempfile |
| Property | შემთხვევითი ოპერაციების შემდეგ "ოპერაცია + Undo = საწყისი ხე" | proptest |
| Crash | პროცესის მოკვლა კოპირების შუაში → აღდგენა ჟურნალიდან | ცალკე test binary |
| Benchmark | listing, დალაგება, ძებნა | criterion |
| UI unit | store-ები, keymap, ფილტრი, მონიშვნა, rename-ის ცხრილი | Vitest + Testing Library |
| E2E | PRD-ის ძირითადი სცენარები რეალურ აპში | WebdriverIO + tauri-driver |
| ხელით | Drag & drop და clipboard Explorer-თან, Outlook-თან, Telegram-თან, AutoCAD-თან; OneDrive; ქსელური დისკი | ჩეკლისტი `docs/`-ში, ყოველ ეტაპზე |

CI (GitHub Actions, windows-latest) ყოველ push-ზე ამოწმებს: rustfmt, clippy (`-D warnings`), nextest, Biome, `tsc`, Vitest და bindings-ის განახლებას. Tag-ზე იკრიბება ინსტალერი და იქმნება release.

## Build, გავრცელება და განახლება

Jerry ვრცელდება NSIS ინსტალერით, მომხმარებლის დონეზე და admin-ის გარეშე. განახლებები v0.1-დან Tauri updater-ით მოდის, რომ ყოველდღიური გამოყენების დროს ახალი ვერსია თავისით დაყენდეს.

- **დეველოპმენტი:** `pnpm tauri dev`; UI-ზე hot reload, Rust-ის ცვლილებაზე ახალი build.
- **Release profile:** `lto = "thin"`, `codegen-units = 1`, `strip = true`. `panic = "unwind"` რჩება, რომ worker-ების panic დაიჭიროს. ახალი exe-ს პირველი გაშვება ~2 წმ-ით ნელია: Defender მას გაშვებამდე ამოწმებს (spike #1), ამიტომ exe-ს ზომაც (ახლა 8.9 MB) მნიშვნელოვანია.
- **ინსტალერი:** NSIS, `installMode: currentUser`, დაყენება `%LOCALAPPDATA%\Programs\Jerry`-ში. WebView2 იყენება `downloadBootstrapper`-ით, თუ Windows 10-ზე არ არის.
- **რეესტრი:** "Open in Jerry" და autostart აპიდან ჩაირთვება პარამეტრებით. Uninstall შლის ყველა ჩანაწერს.
- **განახლება:** `tauri-plugin-updater`, პაკეტები ხელმოწერილია updater-ის გასაღებით. `latest.json` და ფაილები საჯარო repo-ს GitHub Releases-ზეა. შემოწმება გაშვებისას და დღეში ერთხელ; ახალი ვერსიისას ჩანს შეტყობინება ცვლილებების სიით და Update ღილაკით, როგორც სხვა აპებში. ინსტალაცია მხოლოდ მომხმარებლის თანხმობით; შემდეგ Jerry თავიდან იხსნება (tauri-plugin-process) და სესია აღდგება. შემოწმების გამორთვა პარამეტრებშია.
- **Code signing:** Jerry საჯაროა, ამიტომ release build-ები და ინსტალერი ხელმოწერილია code signing სერტიფიკატით, რომ SmartScreen-მა არ გააფრთხილოს. იგივე სერტიფიკატი ხელს აწერს sparse package-ს. საჯარო გამოშვებამდე სახელი და აიკონი სავაჭრო ნიშანზე მოწმდება (PRD).
- **ვერსიები:** SemVer, Conventional Commits, CHANGELOG ავტომატურად commit-ებიდან.
- **პლატფორმა:** მხოლოდ x64; ARM64 build არ იგეგმება.

## Spike და ეტაპები

განვითარება იწყება ერთკვირიანი spike-ით, რომელიც სტეკის შიგნით რისკიან ტექნიკებს ამოწმებს. შემდეგ ეტაპები PRD-ის თანმიმდევრობით მიდის და ყოველი ეტაპის ბოლოს Jerry ყოველდღიურად გამოსაყენებელია.

### Spike (კვირა 0)

| # | შემოწმება | წარმატების კრიტერიუმი |
| --- | --- | --- |
| 1 | ცივი გაშვება | 1 000 ელემენტიანი საქაღალდე ჩანს < 1 წმ-ში process start-იდან |
| 2 | 100 000 ელემენტი | პირველი რიგები < 0.3 წმ, scroll 60 fps, დალაგება < 100 ms |
| 3 | Drag & drop გარეთ | Explorer, Desktop, Outlook (attachment), Telegram, Chrome (upload) და AutoCAD ფაილს იღებს |
| 4 | Drag & drop შიგნით | Explorer-იდან მუშაობს; Outlook-ის attachment ფაილად ინახება; ტაბი, breadcrumb და sidebar სწორად ინათება |
| 5 | Clipboard | Jerry ↔ Explorer copy, cut, paste ორივე მიმართულებით; ჩასმა Outlook-სა და Telegram-ში |
| 6 | Windows options | 7-Zip-ისა და "Send to"-ს ქვემენიუები მუშაობს |
| 7 | Title bar | ორად გაყოფილი ტაბების ზოლი, ფანჯრის გადათრევა, ორმაგი კლიკი, Snap Layouts, Aero Snap, სწორი maximize ზომები |
| 8 | ქართული | ქართული ფაილის სახელების რენდერი სუფთაა; shortcut-ები ქართულ განლაგებაზე მუშაობს |

Spike-ის კოდი ცალკე branch-ზე რჩება. შედეგები და არჩეული ტექნიკები ამ დოკუმენტში ჩაიწერება.

**#1 ცივი გაშვება:** გაზომილია 02.11.2026 ([#11](https://github.com/tshamugia/jerry/issues/11#issuecomment-6565323519)); ცივი გაშვება (ცარიელი ქეში) და თეთრი ციმციმის შემოწმება თვალით ჯერ დარჩა. Jerry-ის საკუთარი ნაწილი თბილ გაშვებაზე ~0.5 წმ-ია (1 000 ელემენტი), აქედან ~70 % WebView2-ის დაყენებაა. Hybrid GPU-იან ლეპტოპზე მძინარე NVIDIA-ს გაღვიძება პირველ კადრს +0.4–1.45 წმ-ს უმატებს (~1.9 წმ) და Jerry-ის კოდიდან ვერ იცილება; ბიუჯეტს სანდოდ მხოლოდ ფონზე დარჩენილი Jerry აკმაყოფილებს (ჩვენება 19–56 ms). გადაწყვეტა «რისკები და კითხვები»-შია.

**#2 100 000 ელემენტი:** გავლილია 02.11.2026 ([#12](https://github.com/tshamugia/jerry/issues/12#issuecomment-6563704652)). პირველი რიგები ~15 ms-ში ჩანს (ბიუჯეტი 300), დალაგების შეცვლა 100 000-ზე ~30 ms-ია (ბიუჯეტი 100), scroll 60 fps-ია release-შიც და debug-შიც. ტექნიკა «მონაცემთა ნაკადი», «დალაგება» და «ფორმატი და შეცდომები» სექციებშია.

**#3 Drag & drop გარეთ:** გავლილია 02.11.2026 ([#13](https://github.com/tshamugia/jerry/issues/13#issuecomment-6563710057)). ექვსივე პროგრამა ფაილს ისე იღებს, როგორც Explorer-იდან; fallback (გამჭვირვალე ფანჯარა) საჭირო არ გახდა. ტექნიკა «Drag & drop», «Clipboard» და «Thread-ების მოდელი» სექციებშია.

**#4 Drag & drop შიგნით:** გავლილია 02.11.2026 ([#14](https://github.com/tshamugia/jerry/issues/14#issuecomment-6565149046)) fallback-ით: OLE target-ს WebView2-ის ფანჯრების ქვეშ ვერ პოულობს, ამიტომ drag-ის დროს გვერდის თავზე გამჭვირვალე ფანჯარა ჩნდება. Explorer, Desktop, Outlook (attachment და `.msg`), Chrome, Edge და Jerry-ის შიგნით გადათრევა მუშაობს; ტაბი, breadcrumb და sidebar ინათება. ტექნიკა «Drag & drop» და «Thread-ების მოდელი» სექციებშია.

**#5 Clipboard:** Explorer-თან გავლილია 02.11.2026 ([#15](https://github.com/tshamugia/jerry/issues/15#issuecomment-6564949126)): Copy, Cut და Paste ორივე მიმართულებით მუშაობს ნამდვილ explorer.exe-სა და jerry.exe-ზე, ქართული სახელებით, MAX\_PATH-ზე გრძელი გზით და Jerry-ის დახურვის შემდეგაც; fallback არ დასჭირდა. ხელით ჯერ დარჩა: ნამდვილი Ctrl+C / X / V და ჩასმა Outlook-სა და Telegram-ში. ტექნიკა «Clipboard» სექციაშია.

### ეტაპები

1. **v0.1 საფუძველი.** Scaffold (Tauri, React, workspace-ები, CI). ფანჯარა ორი პანელით (ნაგულისხმევი) და ტაბებით სათაურის ზოლში; F3 ერთ პანელზე გადართავს. Listing, watcher, დალაგება, Details ხედი, მონიშვნა. ბრძანებების რეესტრი და keymap. Breadcrumbs და მისამართის ზოლი. Sidebar (ფავორიტები, დისკები). ახალი საქაღალდე, გადარქმევა, Recycle Bin. Clipboard. სწრაფი ფილტრი. Status bar. SQLite (პარამეტრები, სესია). ერთი ეგზემპლარი. ლოგები. ინსტალერი და updater.
2. **v0.2 ტრანსფერები.** ტრანსფერების ძრავი, რიგი და პანელი. კონფლიქტების ფანჯარა. ჟურნალი და crash recovery. OLE drag & drop. კოპირება და გადატანა მეორე პანელში (F5 / F6). Undo/Redo და ისტორიის პანელი. შეცდომების მოდელი. Taskbar-ის პროგრესი და toast.
3. **v0.3 ძებნა და Preview.** ძებნის ძრავი და შედეგების ტაბი. Quick Look და preview პანელი. List და Grid. Thumbnail protocol. Go to folder.
4. **v1.0 სრული ვერსია.** Batch rename. ZIP. Command palette. Windows options მენიუ. პარამეტრები და shortcut-ების შეცვლა. Hotkey და autostart. "Open in Jerry". OneDrive. Recycle Bin-ის ხედი. რამდენიმე ფანჯარა და ტაბის გატანა. Open with და "Open in…".
5. **1.0-ის შემდეგ.** შიგთავსში ძებნა. ნაგულისხმევი ფაილ მენეჯერი (Win+E). საქაღალდეების ხე. ინდექსირებული ძებნა მთელ დისკზე Windows Search-ის ინდექსით; საკუთარი MFT ინდექსი admin-ს მოითხოვს და ამიტომ გამორიცხულია.

## რისკები და კითხვები

ყველაზე დიდი რისკი OLE drag & drop-ის WebView2-თან ინტეგრაციაა. ამიტომ ის spike-ში პირველ რიგში მოწმდება.

| რისკი | რას აზიანებს | შემცირება |
| --- | --- | --- |
| OLE drag & drop WebView2-ზე | გადათრევის სცენარები | spike #3 (გარეთ) გავლილია, fallback არ დასჭირდა; spike #4 (შიგნით) გავლილია fallback-ით: გამჭვირვალე ფანჯარა drag-ის დროს. WebView2-ის განახლებამ ეს ქცევა შეიძლება შეცვალოს: E2E ტესტი CI-ში |
| Snap Layouts custom title bar-ზე | Windows 11-ის ჩვეული ქცევა | spike #7; native hit-test |
| Hybrid GPU (Intel + NVIDIA ლეპტოპი) | გაშვება < 1 წმ | WebView2-ის GPU პროცესი მძინარე NVIDIA-ს აღვიძებს: +0.4–1.45 წმ (spike #1), ფლაგებით ვერ აიცილება. გადაწყვეტა ქვემოთ, ღია კითხვებშია |
| მესამე მხარის Shell extension-ის ჩამოკიდება ან crash | thumbnail-ები, context menu | timeout-ები; საჭიროებისას thumbnail-ები ცალკე helper პროცესში |
| OneDrive ფაილის შემთხვევითი ჩამოტვირთვა | ტრაფიკი, ნელი UI | `SIIGBF_INCACHEONLY`; cloud-only შიგთავსი არასოდეს იკითხება; ცალკე ტესტი |
| Shell API-ები 260+ სიმბოლოიან გზებზე | Recycle Bin, context menu | საკუთარი ოპერაციები და წინასწარი გაფრთხილება |
| WebView2 Evergreen-ის განახლებები | ქცევის შეცვლა ჩვენი release-ის გარეშე | E2E ტესტები CI-ში უახლეს runtime-ზე |
| tauri-specta-ს სტაბილურობა | IPC ტიპები | ვერსია scaffold-ისას ფიქსირდება; ალტერნატივა ts-rs |

### კითხვები და პასუხები (01.11.2026)

- [x] გავრცელება: მხოლოდ პირადი გამოყენება, თუ საჯარო და TSGroup-ის კლიენტებისთვისაც? ამაზეა დამოკიდებული code signing და MSIX ("Open in Jerry" Windows 11-ის ახალ მენიუში). საჯარო იქნება
- [x] Repo private იქნება? მაშინ updater-ის `latest.json` და ფაილები საჯარო ადგილას უნდა იყოს (ცალკე releases repo ან static hosting). არა რეპო იქნება ფაბლიკი
- [x] Replace-ით ჩანაცვლებული ფაილი Recycle Bin-ში იგზავნება: ვეთანხმებით? PRD-ში ეს ქცევა აღწერილი არ არის. კი ვეთანხმები
- [x] DWG preview: საკმარისია Windows-ის thumbnail (AutoCAD ან DWG TrueView დაყენებულია?), თუ 1.0-ის შემდეგ ფაილში ჩაშენებული preview-ს თავად ამოვიღებთ? DWG preview არ არის აუცილებელი, დატოვე windows thumbnail
- [x] რამდენიმე ფანჯარა და ტაბის გატანა v1.0-ში დავტოვოთ? PRD-ში ეტაპი მითითებული არ არის. კი ვერსია ერთში იყო.

### ღია კითხვები (02.11.2026)

- [ ] გაშვება hybrid GPU-ზე (spike #1): NVIDIA-ს გაღვიძების გამო პირველი რიგი ~1.9 წმ-ში ჩანს. (ა) ბიუჯეტში გამონაკლისად ჩავწეროთ (desktop-ზე და მხოლოდ ინტეგრირებულ GPU-ზე ~0.5 წმ), თუ (ბ) Jerry ფონზე დარჩეს: Windows-ში შესვლისას დამალულად ეშვება, ბოლო ფანჯრის დახურვა მას მალავს, გაშვება კი მზა ფანჯარას 19–56 ms-ში აჩენს (~120 MB მუდმივად)? Spike-ის რეკომენდაცია: (ბ) პარამეტრად, ნაგულისხმევად ჩართული; v0.1-ში საკმარისია "დახურვა = დამალვა" და single instance.
- [x] ARM64 build საჭიროა თუ არა? არა არ არის საჭირო
