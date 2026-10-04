# MergeMark Windows beta: notes for testers

MergeMark turns past-paper PDFs into a bank of question cards you can filter, build worksheets from, and export. Everything runs on your computer: it sends nothing to any server or AI service.

## Install
- Run `MergeMark_0.9.4_x64-setup.exe`. It installs for your Windows account only, so it needs no administrator rights.
- The installer is not code-signed yet, so Windows SmartScreen may say "Windows protected your PC". Choose **More info**, then **Run anyway**.
- Your library is stored in `%APPDATA%\com.mergemark.app`. Uninstall from **Settings → Apps** in Windows.

## Before your first import
Every card is filed under a subject and a module. The app starts with some maths subjects. For anything else, open **Settings → Curriculum Taxonomy**, add the subject, and add at least one module to it.

## Importing a question paper
Open **Import PDF**, choose the subject and module, then drag a question-paper PDF onto the drop area (or click to browse).

- **Cards marked REVIEW need checking against the paper.** The import flags any card it can't fully verify: a symbol it couldn't identify, marks that don't add up, a question whose end it couldn't find, or text read by OCR.
- **Cards without REVIEW passed every check, but can still be wrong.** If you find one, please report it: these are the most useful reports.
- **Scanned papers** are read by Windows' own text recognition. Every card from a scan is marked REVIEW, because OCR misreads characters (for example "m s-l" for m s⁻¹).
- A paper that prints no question numbers gives no cards.

## Mark schemes: not supported in this beta
**Import Mark Scheme** is still there, but MergeMark's local reader can't read most schemes yet. When it can't, it says so and imports nothing.

## Worksheets and flashcards
- **Compile PDF** needs a LaTeX installation such as MiKTeX. The first compile may pause while MiKTeX downloads packages. If it then reports that pdflatex did not finish, compile again.
- Worksheets and answer keys are saved to your **Downloads** folder.
- **Flashcards → Export to CSV** currently includes print-layout commands (such as `\needspace` and `\hfill`) in the card text. Known issue.

## What to report
For each problem, please send:
- the paper's file name and the question number;
- what is wrong (missing text, wrong symbol, wrong marks, a diagram missing or cropped badly);
- a screenshot of the card and, if you can, of the page in the PDF.

The most valuable report is a card with **no REVIEW badge** that is still wrong,
because nothing in the app catches those.

Send reports by opening an issue at
<https://github.com/badma025/MergeMark/issues/new>. Please don't attach whole
exam papers — the file name and question number are enough.
