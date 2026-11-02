# ADR-0002: Modal loop-ები Jerry-ის საკუთარი message-only ფანჯრით

- **სტატუსი:** მიღებულია, 02.11.2026; დაზუსტდა 03.11.2026
- **წყარო:** spike #4 ([#14](https://github.com/tshamugia/jerry/issues/14#issuecomment-6565149046)), spike #6 ([#16](https://github.com/tshamugia/jerry/issues/16#issuecomment-6587473737)), spike #5 ([#15](https://github.com/tshamugia/jerry/issues/15#issuecomment-6564949126)); Tech PRD «Thread-ების მოდელი»

## კონტექსტი

OLE-ის modal loop-ები (`SHDoDragDrop`, `TrackPopupMenuEx`) მთავარ STA thread-ზე უნდა გაეშვას, WebView2-ის IPC callback-ის გარეთ. Tech PRD-ში ამისთვის Tauri-ს `run_on_main_thread` იყო. მისი closure tao-ს callback-ში სრულდება, და სანამ ის მუშაობს, tao სხვა thread-ებიდან გამოგზავნილ მოვლენებს აკავებს (ისინი event loop proxy-ით მიდის). ამიტომ Rust → გვერდი მოვლენები (`dnd:over`, მენიუს მოვლენები) loop-ის დასრულებამდე არ მოდის. Spike #6-ში `run_on_main_thread`-ით მოვლენა მენიუს დახურვამდე არ მოვიდა, საკუთარი ფანჯრით კი 6–15 ms-ში მოვიდა.

## გადაწყვეტილება

- `jerry-win`-ს აქვს საკუთარი message-only ფანჯარა (`HWND_MESSAGE`) მთავარ thread-ზე და `run_on_main`, რომელიც closure-ს ამ ფანჯარას უგზავნის.
- ყოველი modal loop (drag, context menu) `async` ბრძანებიდან `run_on_main`-ით ეშვება.
- Tauri-ს `run_on_main_thread` modal loop-ისთვის არ გამოიყენება. მოკლე, არა-modal გამოძახებები (clipboard) იმავე გზით მიდის, რომ მთავარი thread-ის COM-ს ერთი შესასვლელი ჰქონდეს.

## შედეგები

- Drag-ისა და მენიუს დროს მოვლენები გვერდამდე აღწევს: `dnd:over` round trip მედიანა 4–5 ms-ია.
- მთავარი thread-ის COM სამუშაოს ერთი ადგილი აქვს `jerry-win`-ში.
- Jerry-ს საკუთარი window proc და რიგი სჭირდება. Closure-ში panic window proc-ს არ უნდა გასცდეს (`catch_unwind`), რადგან FFI-ს გავლით unwind აკრძალულია.

## განხილული ალტერნატივები

- **`run_on_main_thread`:** მოვლენებს loop-ის დასრულებამდე აკავებს.
- **Sync ბრძანება:** WebView2-ის IPC callback-ში სრულდება, სადაც modal loop re-entrancy-ს იწვევს.
- **ცალკე STA thread საკუთარი ფანჯრით:** OLE drag იმ thread-ზე უნდა ეშვებოდეს, რომელიც Jerry-ის ფანჯარას ფლობს.
