# MergeMark 1.0: notes for users

MergeMark turns past-paper PDFs into a bank of question cards you can filter, build worksheets from, and export. Everything runs on your computer: imports send nothing to any server or AI service.

## Install
Download the installer for your computer from the project's Releases page. Neither build is code-signed yet, so the first launch needs one extra step.

**Windows.** Run `MergeMark_1.0.0_x64-setup.exe`. It installs for your Windows account only, so it needs no administrator rights. Windows SmartScreen may say "Windows protected your PC": choose **More info**, then **Run anyway**. Uninstall from **Settings → Apps**.

**macOS** (Apple Silicon and Intel, macOS 11 or later). Open `MergeMark_1.0.0_universal.dmg` and drag MergeMark into Applications. macOS will refuse to open it the first time, because the app is not notarised by Apple. This cannot be clicked past; it needs a setting:
1. Open MergeMark once, and choose **Done** (or **OK**) when macOS says it cannot be opened.
2. Open **System Settings → Privacy & Security** and scroll down to **Security**.
3. Next to "MergeMark was blocked to protect your Mac", choose **Open Anyway**, and confirm with your password.

From then on MergeMark opens normally.

Your library is stored in `%APPDATA%\com.mergemark.app` on Windows and `~/Library/Application Support/com.mergemark.app` on macOS. When a new version first opens it, it keeps a copy beside it (`mergemark.db.bak-<version>`).

## Updates
MergeMark checks for a newer version when it starts, and on request from **Settings → Backup & Maintenance → Updates**, where you can also turn off the check at start-up. The check asks the project's GitHub releases for the newest version's details; it carries nothing about your library or papers. An update installs only when you choose **Install and restart**, and only if it is signed with MergeMark's key.

Coming from a 0.9 beta? It cannot update itself to 1.0, because 1.0 is signed with a new key. Download 1.0 from the Releases page and install it over the beta: your library is kept, and a backup of it is made when 1.0 first opens it.

## Before your first import
Every card is filed under a subject and a module. The app starts with some maths subjects. For anything else, open **Settings → Curriculum Taxonomy**, add the subject, and add at least one module to it.

## Importing a question paper
Open **Import PDF**, choose the subject and module, then drag a question-paper PDF onto the drop area (or click to browse).

- **Cards marked REVIEW need checking against the paper.** The import flags any card it can't fully verify: a symbol it couldn't identify, marks that don't add up, a question whose end it couldn't find, or text read by OCR.
- **Cards without REVIEW passed every check, but can still be wrong.** If you find one, please report it: these are the most useful reports.
- **Scanned papers** are read by your computer's own text recognition. Every card from a scan is marked REVIEW, because OCR misreads characters (for example "m s-l" for m s⁻¹).
- A paper that prints no question numbers gives no cards.

## Checking a card against its paper
Each imported card has a source button (the page with a magnifier, at the top right of the card). It shows the pages of the PDF the question was read from, so you can compare the two. Zoom in with the + and − buttons.

- If you have moved or renamed the PDF since importing it, the card says so. Choose **Find the PDF...** and pick the file in its new place: every card from that paper follows it. MergeMark accepts the file only if it is the same PDF.
- If the file at that place has changed (for example, a different download of the paper), the card says so rather than showing pages that may not match. Find the original, or import the paper again.
- Cards imported by a version before 1.0 record no paper until their paper is imported again.

## Mark schemes
MergeMark 1.0 does not import mark schemes. To keep a question's answer, open its card, type the answer into **Mark Scheme Answer**, and save.

## Worksheets and flashcards
- **Compile PDF** needs a LaTeX installation such as MiKTeX. If MergeMark can't find one, it says so and links to MiKTeX. The first compile may pause while MiKTeX downloads packages.
- Worksheets, answer keys and flashcard exports are saved to your **Downloads** folder.
- **Flashcards → Export to CSV** gives each card's text as the card shows it; a diagram appears as `[Diagram]`.

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
