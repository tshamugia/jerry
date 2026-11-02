# Architecture Decision Records

აქ ინახება გადაწყვეტილებები, რომლებმაც Tech PRD-ის თავდაპირველი ტექნიკა შეცვალა, მათი მიზეზით. Tech PRD აღწერს, როგორ არის აწყობილი Jerry ახლა. ADR კი ინახავს, რატომ აირჩა ეს გზა და რა უარვყავით.

| # | გადაწყვეტილება | სტატუსი | წყარო |
| --- | --- | --- | --- |
| [0001](0001-binary-listing-batches.md) | Listing-ის batch-ები ბინარულ სვეტებად | მიღებულია, 02.11.2026 | spike #2 ([#12](https://github.com/tshamugia/jerry/issues/12)) |
| [0002](0002-modal-loops-on-own-window.md) | Modal loop-ები Jerry-ის საკუთარი message-only ფანჯრით | მიღებულია, 02.11.2026 | spike #4 ([#14](https://github.com/tshamugia/jerry/issues/14)), #6 ([#16](https://github.com/tshamugia/jerry/issues/16)) |
| [0003](0003-drop-target-overlay.md) | Drop target გამჭვირვალე ფანჯარაზე drag-ის დროს | მიღებულია, 02.11.2026 | spike #4 ([#14](https://github.com/tshamugia/jerry/issues/14)) |
| [0004](0004-title-bar-input-window.md) | Title bar: CSS `app-region` და caption ღილაკების input ფანჯარა | მიღებულია, 02.11.2026 | spike #7 ([#17](https://github.com/tshamugia/jerry/issues/17)) |
| [0005](0005-keep-running-in-background.md) | Jerry ფონზე რჩება | მიღებულია, 03.11.2026 | spike #1 ([#11](https://github.com/tshamugia/jerry/issues/11)) |

## ახალი ADR

ფაილი `NNNN-short-slug.md`, ნომერი ზედიზედ. ცვლილება ჯერ ADR-ში ან Tech PRD-ში იწერება, შემდეგ კოდში (`CLAUDE.md`). ძველი ADR არ იშლება: ახალი მას ცვლის და სტატუსში წერია "ჩანაცვლებულია ADR-NNNN-ით".

```markdown
# ADR-NNNN: სათაური

- **სტატუსი:** შემოთავაზებული | მიღებულია, DD.MM.YYYY | ჩანაცვლებულია ADR-NNNN-ით
- **წყარო:** issue, spike, Tech PRD-ის სექცია

## კონტექსტი
რა პრობლემა იყო და რა ვიცოდით.

## გადაწყვეტილება
რას ვაკეთებთ.

## შედეგები
რას ვიგებთ და რას ვიხდით.

## განხილული ალტერნატივები
რა უარვყავით და რატომ.
```
