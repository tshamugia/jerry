# ADR-0001: Listing-ის batch-ები ბინარულ სვეტებად

- **სტატუსი:** მიღებულია, 02.11.2026
- **წყარო:** spike #2 ([#12](https://github.com/tshamugia/jerry/issues/12#issuecomment-6563704652), branch `spike/12-100k-items`); Tech PRD «ფორმატი და შეცდომები», «მონაცემთა ნაკადი»

## კონტექსტი

Tech PRD-ში listing-ის batch იყო "სვეტების მასივები", ფორმატის დაზუსტების გარეშე. JSON-ით (specta-ს ტიპები) 100 000 ელემენტის encode და გაგზავნა listing thread-ზე 51.7 ms-ს იკავებს release-ში, debug-ში კი 390 ms-ს. specta `f64`-ს `number | null`-ად აქცევს, `u64`-ს კი JS ზუსტად ვერ ინახავს BigInt-ის გარეშე.

## გადაწყვეტილება

- თითო batch ერთი `ArrayBuffer`-ია, Tauri `Channel`-ით, little-endian სვეტებად: 16-ბაიტიანი header `[kind, count, firstId, nameUnits]`, შემდეგ `f64 size[]`, `f64 modified[]`, `u32 attributes[]`, `u32 nameEnd[]` და სახელები UTF-16-ად.
- ერთ ნაკადში ოთხი შეტყობინებაა (პირველი `u32` = kind): batch, order, done, error. დალაგების შეცვლა (`listing_sort`) იმავე order შეტყობინებას აბრუნებს `tauri::ipc::Response`-ით.
- ზომა `f64`-ია (ზუსტია 2^53 ბაიტამდე), თარიღი `f64` ms Unix epoch-იდან. თარიღით დალაგება Rust-ში ზუსტ `u64` FILETIME-ზე ხდება.
- Rust-ის `Bytes` ტიპი ახორციელებს `IpcResponse`-ს და bindings-ში `ArrayBuffer`-ად ჩანს.
- პირველი batch 256 ელემენტზე იგზავნება, შემდეგები 50 ms-ში ერთხელ, დანარჩენი ბოლოს.

## შედეგები

- encode + send 100 000-ზე 4.9 ms-ია (JSON-ით 51.7 ms). პირველი რიგები ~15 ms-ში ჩანს, დალაგების შეცვლა ~30 ms-ია, სრული დალაგებული სია 115–210 ms.
- JS-ში typed array view-ები კოპირების გარეშე იქმნება, სახელებს ერთი `TextDecoder("utf-16le")` შიფრავს.
- ფორმატი თვითაღწერადი არ არის: decoder `src/ipc`-ში ხელით იწერება და Rust-ის layout-ს უნდა ემთხვეოდეს. ორივე მხარეს unit ტესტი სჭირდება, ახალი სვეტი (მაგ. აიკონის გასაღები) layout-ს ცვლის.
- Debug-ში ბაიტების ნახვა JSON-ზე რთულია.

## განხილული ალტერნატივები

- **JSON specta-ს ტიპებით:** 10-ჯერ ნელი encode, `number | null`, `u64`-ის პრობლემა.
- **MessagePack ან bincode:** ახალი დამოკიდებულება, რომელიც Tech PRD-ში არ არის, და JS-ში მაინც კოპირება.
