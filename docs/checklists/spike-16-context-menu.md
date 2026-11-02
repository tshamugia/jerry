# Spike #16: Windows options (Explorer-ის context menu) · ხელით შემოწმება

მიზანი: 7-Zip-ისა და "Send to"-ს ქვემენიუები იხსნება და ბრძანებები სრულდება. ავტომატურად გავლილია ნამდვილ jerry.exe-ში (`src-tauri/examples/menu_e2e.ps1`) და ცალკე probe-ში (`crates/jerry-win/examples/menu_matrix.ps1`): Send to და WinRAR (7-Zip-ის ნაცვლად) ერთ ფაილზე, რამდენიმე ფაილზე, საქაღალდეზე, ქართულ სახელსა და MAX_PATH-ზე გრძელ გზაზე; Properties, Rename, Esc და სხვადასხვა საქაღალდის ფაილები. აქ რჩება ის, რასაც მხოლოდ ნამდვილი მაუსი, კლავიატურა და შენი თვალი ამოწმებს, და 7-Zip, რომელიც ამ კომპიუტერზე არ არის.

## მომზადება

1. **7-Zip** (წარმატების კრიტერიუმი): დააყენე `winget install 7zip.7zip` (ან 7-zip.org-იდან). შემდეგ ავტომატური შემოწმება ერთი ბრძანებით, worktree-ში:
   `powershell -ExecutionPolicy Bypass -File crates\jerry-win\examples\menu_matrix.ps1 -Archiver 7-Zip`
   ბოლოს ყველა ხაზი PASS უნდა იყოს.
2. გაუშვი `D:\03_GITHUB\jerry\.claude\worktrees\modest-bun-b8923d\target\release\jerry.exe`. Jerry აჩვენებს სატესტო საქაღალდის `%TEMP%\jerry-spike-16` ელემენტებს და ღილაკებს (One file, Several files, Folder, Long path, Mixed folders).
3. ქვემოთ Jerry წერს, რა აირჩიე და მოვიდა თუ არა მოვლენები მენიუს გახსნილობისას ("Events during the menu: yes"), JSON-ში კი დროებს. ლოგი: `%TEMP%\jerry-spike-16.log`.
4. შედარებისთვის გახსენი იგივე საქაღალდე Explorer-ში (`%TEMP%\jerry-spike-16`): მარჯვენა კლიკი → "Show more options".

## შემოწმებები

| # | რა გავაკეთო | მოსალოდნელი | შედეგი |
| --- | --- | --- | --- |
| 1 | Jerry-ში მარჯვენა კლიკი "ნახაზი — სართული 1.dwg"-ზე | მენიუ კურსორთან ჩნდება, იგივე პუნქტებით, რაც Explorer-ის "Show more options"-ში, აიკონებით. მიუთითე, თუ რამე აკლია ან მეტია | |
| 2 | 7-Zip → ქვემენიუ → Add to "ნახაზი — სართული 1.7z" | ქვემენიუ იხსნება აიკონებით, არქივი ჩნდება საქაღალდეში, ქართული სახელით | |
| 3 | შექმნილ .7z-ზე: 7-Zip → Open archive, შემდეგ Extract Here | 7-Zip File Manager იხსნება; ამოარქივება მუშაობს | |
| 4 | Send to → Compressed (zipped) folder; Send to → Documents | zip ჩნდება იმავე საქაღალდეში; Documents-ში ასლი ჩნდება (შემდეგ წაშალე) | |
| 5 | Ctrl+click-ით 2–3 ფაილი, მარჯვენა კლიკი → 7-Zip → Add to "jerry-spike-16.7z" | ერთი არქივი ყველა მონიშნულით | |
| 6 | "პროექტი" საქაღალდე: Open in Terminal; Send to → Compressed folder | Terminal იხსნება ამ საქაღალდეში; zip ჩნდება | |
| 7 | Shift + მარჯვენა კლიკი ფაილზე | Explorer-ის Shift-მენიუს იგივე დამატებითი პუნქტები | |
| 8 | კლავიატურა: Tab-ით რიგზე, Menu ღილაკი ან Shift+F10; ისრები, → ქვემენიუსთვის, Enter, Esc; ასო-mnemonic (მაგ. N "Send to"-სთვის) | მენიუ რიგის ქვეშ ჩნდება, ყველაფერი კლავიატურით მუშაობს | |
| 9 | Open with → ქვემენიუ → Notepad (ან სხვა პროგრამა) | პროგრამების სია აიკონებით; ფაილი იხსნება | |
| 10 | Rename, Delete, Cut, Copy | Spike-ში არაფერი ხდება, ქვემოთ წერია "Jerry's own verb, not invoked" (v1.0-ში Jerry-ის საკუთარი ბრძანებები) | |
| 11 | მენიუს გარეთ კლიკი, Alt+Tab მენიუს გახსნილობისას | მენიუ იხურება, Jerry პასუხობს | |
| 12 | მენიუს ფერი Windows-ის მუქ და ღია რეჟიმში (Settings → Personalization → Colors, Jerry-ის გადატვირთვით) | მუქ რეჟიმში მუქი, ღიაში ღია, როგორც Explorer-ში | |
| 13 | სხვა მასშტაბის მონიტორზე (თუ გაქვს): Jerry გადაიტანე და მარჯვენა კლიკი | მენიუ კურსორთან, მკვეთრი | |

## შედეგის მოწოდება

თითოეულ ხაზზე: მუშაობს / არ მუშაობს / ნაწილობრივ, და მოკლე შენიშვნა. თუ რამე არ მუშაობს, გამომიგზავნე `%TEMP%\jerry-spike-16.log`-ის ბოლო ხაზები.
