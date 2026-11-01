# Spike #15: Clipboard Jerry ↔ Explorer · ხელით შემოწმება

მიზანი: Jerry-ის Copy, Cut და Paste ისე მუშაობს, როგორც Explorer-ის, ორივე მიმართულებით. Outlook და Telegram Jerry-ის ფაილს იღებს. Explorer-ის მხარე ავტომატურად უკვე შემოწმდა (`crates/jerry-win/examples/clip_probe.rs`). აქ რჩება ნამდვილი კლავიატურა, Outlook და Telegram.

## მომზადება

1. გაუშვი `target\release\jerry.exe` (branch `spike/15-clipboard`). Jerry აჩვენებს სატესტო საქაღალდეს `%TEMP%\jerry-spike-15`.
2. ფანჯარა სათაურის ზოლით გადაადგილდება, ✕ ხურავს. Win+← / Win+→ Explorer-ის გვერდით დააყენებს.
3. მონიშვნა: კლიკი, Ctrl+კლიკი, Shift+კლიკი, Ctrl+A. Shortcut-ები: Ctrl+C, Ctrl+X, Ctrl+V (ან ღილაკები ზემოთ). ჩასმა ყოველთვის ამ საქაღალდეში ხდება.
4. ქვემოთ Jerry წერს ბოლო მოქმედების შედეგს და clipboard-ის მდგომარეობას (რამდენი ფაილი, Copy თუ Cut, ვისია). ლოგი: `%TEMP%\jerry-spike-15.log`.
5. შეამოწმე ქართულ განლაგებაზეც: Ctrl+C / X / V უნდა მუშაობდეს.

## სცენარები

| # | სცენარი | რა გავაკეთო | მოსალოდნელი | შედეგი |
| --- | --- | --- | --- | --- |
| 1 | Jerry → Explorer, Copy | Jerry-ში მონიშნე 2 ფაილი და `საქაღალდე`, Ctrl+C. Explorer-ში სხვა საქაღალდეში Ctrl+V | სამივე ჩაისმება, ქართული სახელები სწორია, Jerry-ში ფაილები რჩება | |
| 2 | Jerry → Explorer, Cut | Jerry-ში მონიშნე ფაილი, Ctrl+X. Explorer-ში Ctrl+V | Jerry-ში ფაილი ნახევრად გამჭვირვალეა, სანამ ჩასვამ. ჩასმის შემდეგ ფაილი Explorer-შია, Jerry-ის სიიდან ქრება. Explorer-ში მეორე Ctrl+V აღარაფერს სვამს | |
| 3 | Explorer → Jerry, Copy | Explorer-ში მონიშნე 2–3 ფაილი (ქართული სახელითაც), Ctrl+C. Jerry-ში Ctrl+V | ჩაისმება, Jerry წერს "Copied … from another app (Preferred DropEffect 5)" | |
| 4 | Explorer → Jerry, Cut | Explorer-ში ფაილი, Ctrl+X (Explorer-ში ნახევრად გამჭვირვალდება). Jerry-ში Ctrl+V | ფაილი გადმოდის Jerry-ში და Explorer-იდან ქრება. Explorer-ში Paste ამის შემდეგ ნაცრისფერია | |
| 5 | Jerry-ის დახურვა | Jerry-ში ფაილი, Ctrl+C. დახურე Jerry (✕). Explorer-ში Ctrl+V | ფაილი ჩაისმება, თუმცა Jerry აღარ არის გაშვებული | |
| 6 | ამოჭრის გაუქმება | Jerry-ში Ctrl+X, შემდეგ სხვა პროგრამაში დააკოპირე ტექსტი | Jerry-ში ნახევრად გამჭვირვალე სტილი ქრება | |
| 7 | Outlook (attachment) | ახალი წერილი. Jerry-ში PDF და TXT, Ctrl+C. წერილში Ctrl+V | ორივე მიმაგრდება სწორი სახელით (გაგზავნა საჭირო არ არის). მიუთითე classic თუ new Outlook | |
| 8 | Telegram | Jerry-ში PDF (სურვილისამებრ 2–3 ფაილი), Ctrl+C. ჩატში Ctrl+V | ჩნდება გაგზავნის ფანჯარა ფაილებით და სწორი სახელებით. შეგიძლია Cancel | |

## შედეგის მოწოდება

თითოეულ ხაზზე: მუშაობს / არ მუშაობს / ნაწილობრივ და მოკლე შენიშვნა, განსაკუთრებით თუ რამე Explorer-ისგან განსხვავებულად იქცევა. თუ რამე არ მუშაობს, გამომიგზავნე `%TEMP%\jerry-spike-15.log`-ის ბოლო ხაზები.
