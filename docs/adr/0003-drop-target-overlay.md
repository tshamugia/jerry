# ADR-0003: Drop target გამჭვირვალე ფანჯარაზე drag-ის დროს

- **სტატუსი:** მიღებულია, 02.11.2026 (fallback)
- **წყარო:** spike #4 ([#14](https://github.com/tshamugia/jerry/issues/14#issuecomment-6565149046), branch `spike/14-ole-drag-in`); Tech PRD «Drag & drop»

## კონტექსტი

Tech PRD-ში Jerry-ის `IDropTarget` WebView2-ის ფანჯარაზე უნდა ყოფილიყო. კურსორის ქვეშ კი ყოველთვის WebView2-ის browser პროცესის ფანჯარაა (`Chrome_WidgetWin_1`, `Chrome_RenderWidgetHostHWND`), და OLE მისგან Jerry-ის ფანჯრებამდე ზემოთ არ ადის. Jerry-ის ფანჯარაზე რეგისტრირებული target გვერდის თავზე არასოდეს გამოიძახება, Tauri-ს ჩაშენებული drop handler-იც არა.

## გადაწყვეტილება

- `IDropTarget` გამჭვირვალე owned popup-ზეა (layered, alpha 1/255, არ აქტიურდება), რომელიც გვერდის თავზე მხოლოდ drag-ის დროს ჩნდება.
- ტაიმერი (50 ms, დაჭერილი ღილაკისას 16 ms) მას აჩენს, როცა Jerry-ს გარეთ დაჭერილი ღილაკით კურსორი Jerry-ზეა ან Jerry-ის საკუთარი drag მიდის.
- `dragDropEnabled: false`; WebView2-ის `AllowExternalDrop` ითიშება, რომ გვერდმა ფაილი არ გახსნას.
- სამიზნეს UI პოულობს: Rust აგზავნის `dnd:over {seq, x, y, keys}`-ს, UI `elementFromPoint`-ით პოულობს საქაღალდეს, ტაბს, breadcrumb-ს ან sidebar-ს და `dnd_answer`-ით აბრუნებს.

## შედეგები

- ფაილს იღებს Explorer-იდან, Desktop-იდან, Outlook-იდან (attachment და `.msg`), Chrome-იდან, Edge-იდან და Jerry-ის შიგნით. Drag image ჩანს `IDropTargetHelper`-ით.
- ტაიმერი უქმ მდგომარეობაშიც მუშაობს (50 ms).
- ქცევა WebView2-ის ფანჯრების სტრუქტურაზეა დამოკიდებული, და Evergreen-ის განახლებამ ის შეიძლება შეცვალოს: E2E ტესტი CI-ში უახლეს runtime-ზე (#63).
- Overlay drag-ის შემდეგ ხილული არ უნდა დარჩეს, თორემ კლიკებს შთანთქავს.

## განხილული ალტერნატივები

- **`IDropTarget` Jerry-ის ფანჯარაზე:** გვერდის თავზე არასოდეს გამოიძახება.
- **Tauri-ს drop handler:** virtual files (Outlook, ბრაუზერი) და drop effect-ის მართვა არ აქვს.
- **WebView2-ის საკუთარი drop:** ფაილს გვერდში ხსნის, OLE-ის მოქმედებების გარეშე.
