# Spike #17: title bar და Snap Layouts, ხელით შემოწმება

Build: `pnpm tauri build --no-bundle` → `target\release\jerry.exe` (branch `spike/17-title-bar`). მარჯვენა სვეტი («Title bar spike») ფანჯრის მიმდინარე მდგომარეობას აჩვენებს: hover, pressed, maximized, active, Mica, Snap Layouts, მასშტაბი, ზომა.

თითოეულ პუნქტს ✅ / ❌ და მოკლე შენიშვნა.

## A. გარეგნობა

1. გაშვება: თეთრი ციმციმი არ არის. 40 px title bar, ტაბები, caption ღილაკები ზედა მარჯვენა კუთხეში. Title bar და sidebar Mica-ზეა (ფონის ფერი ოდნავ ჩანს), პანელები გაუმჭვირვალეა. მარჯვენა სვეტში: Mica yes, Snap Layouts yes.
2. Windows-ის ღია/მუქი თემის შეცვლა (Settings → Personalization → Colors): ფერები და Mica მიჰყვება.

## B. ცარიელი ზოლი: native caption (CSS `app-region: drag`, WebView2)

3. ცარიელი ადგილიდან გადათრევა («Jerry»-ს კუთხე, ორივე `+`-ის მარჯვნივ, caption ღილაკების მარცხნივ): ფანჯარა მოძრაობს.
4. ზედა კიდესთან → maximize; მარცხენა/მარჯვენა კიდესთან → ნახევარი; კუთხესთან → მეოთხედი. Maximized ფანჯრის ზოლით ქვემოთ გამოთრევა → restore და კურსორს მიჰყვება.
5. ცარიელ ზოლზე ორმაგი კლიკი → maximize; კიდევ ერთი → restore.
6. ცარიელ ზოლზე მარჯვენა კლიკი → system menu (Restore / Move / Size / Minimize / Maximize / Close).
7. ტაბები გადასათრევი არეა არ არის: ტაბზე კლიკი (აქტიურდება), × (იხურება), `+` (ემატება), pinned ტაბი.
8. body-ში პანელების გამყოფის გადაწევა: ტაბების ზოლის გაყოფა მას მიჰყვება.

## C. Caption ღილაკები (native hit-test)

9. Minimize / maximize / close-ზე hover: ფონი ინთება (close წითლდება), «Hovered» მიჰყვება, გასვლისას ქრება.
10. Maximize-ზე ~1 წმ hover → **Snap Layouts flyout**; ზონის არჩევა → ფანჯარა იქ ჯდება.
11. Minimize / Maximize (glyph იცვლება Restore-ით) / Restore / Close მუშაობს.
12. Maximize-ზე დაჭერა, გასრიალება და აშვება ღილაკის გარეთ → არაფერი ხდება.
13. სხვა აპზე კლიკი: glyph-ები ფერმკრთალდება, «Active window: no».

თუ 9–12 საერთოდ არ რეაგირებს (hover არ ინთება, კლიკი არაფერს აკეთებს): დახურე Jerry, გაუშვი `JERRY_PLAIN_INPUT=1`-ით (PowerShell: `$env:JERRY_PLAIN_INPUT=1; & "<path>\jerry.exe"`), გაიმეორე 9–12 და ჩაწერე, ჩანს თუ არა სამივე ღილაკი ამ რეჟიმში.

## D. კლავიატურა

14. Win+↑ maximize, Win+↓ restore/minimize, Win+←/→ snap, Win+Z Snap Layouts.

## E. Maximize-ის ზომა

15. Maximized: ზუსტად ავსებს სამუშაო არეს; არაფერი გადის ეკრანს გარეთ (ზედა რიგი და მარჯვენა ღილაკები მთლიანად ჩანს); taskbar ჩანს და დაფარული არ არის.
16. Taskbar auto-hide ჩართული → maximize → თაგვი taskbar-ის კიდესთან → taskbar Jerry-ს თავზე ამოდის.

## F. Resize და DPI

17. Resize ყველა კიდიდან და კუთხიდან, მათ შორის ზედა კიდიდან ტაბების და caption ღილაკების თავზე.
18. მასშტაბის შეცვლა Jerry-ს გახსნისას (Settings → Display, მაგ. 100 → 125 %) ან გადატანა სხვა მასშტაბის მონიტორზე: ზომები სწორია, hover/კლიკი ისევ სწორ ღილაკზე ხვდება, «Scale» იცვლება.

## G. Windows 10-ის სახე

19. Jerry-ს გაშვება `JERRY_NO_MICA=1`-ით (PowerShell: `$env:JERRY_NO_MICA=1; & "<path>\jerry.exe"`): ფონი ერთფეროვანი რუხია, დანარჩენი მუშაობს.
