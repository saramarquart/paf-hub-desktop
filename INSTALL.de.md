# Planet A Foods Desktop: Installation (Kurzanleitung)

Der App-Hub (<https://hub.planet-a-foods.com/>) als Desktop-App: ein Fenster mit
Tabs für alle internen Tools. Gleiche Logins, gleiche Daten wie im Browser. Die
App braucht eine Internetverbindung. *English: [README.md](README.md).*

Den Installer gibt es auf der Seite [Releases](../../releases) (neueste Version
oben). Die Dateinamen unten gelten für Version 0.2.6.

## Mac

1. **`Planet.A.Foods_0.2.6_universal.dmg`** herunterladen (für Apple-Silicon- und Intel-Macs).
2. Öffnen und **Planet A Foods** in **Programme** ziehen.
3. **Nur beim ersten Mal:** **Terminal** öffnen (⌘ Leertaste, *Terminal*, Enter),
   diese Zeile einfügen und Enter drücken:

   ```
   sudo xattr -dr com.apple.quarantine "/Applications/Planet A Foods.app"
   ```

   Dann dein Mac-Passwort eingeben. Beim Tippen erscheint nichts, das ist normal.
   Dafür brauchst du Admin-Rechte; falls dein Konto keine hat, bitte die IT fragen.
4. **Planet A Foods** aus Programme öffnen.

Ohne Schritt 3 meldet macOS, die App *„ist beschädigt und kann nicht geöffnet
werden"* oder *„Apple konnte nicht überprüfen …"*. Sie ist nicht beschädigt. Der
Grund: Wir lassen die App nicht kostenpflichtig von Apple beglaubigen. Schritt 3
ausführen und die App erneut öffnen. Installiert die IT die App per
Geräteverwaltung (**`Planet-A-Foods_0.2.6_universal.pkg`**), entfällt Schritt 3.

## Windows

1. **`Planet.A.Foods_0.2.6_x64-setup.exe`** herunterladen.
2. Ausführen, dann **Planet A Foods** über das Startmenü öffnen.

Erscheint **„Der Computer wurde durch Windows geschützt"** (SmartScreen): auf
**Weitere Informationen** → **Trotzdem ausführen** klicken. Das kommt nur bei der
Installation, weil unsere Installer kein kommerzielles Zertifikat haben. Installiert
die IT die App für dich, erscheint die Meldung nicht.

## Anmelden

Die Hub-Seite selbst braucht keine Anmeldung. Öffnest du eine App, die eine braucht:

1. **Dein Browser öffnet sich** mit der Anmeldeseite dieser App.
2. Dort wie gewohnt mit dem Google-Arbeitskonto anmelden.
3. Der Browser-Tab zeigt **„Signed in — you can close this tab"** und **Planet A
   Foods kommt angemeldet wieder nach vorn**, im selben Tab. Den Browser-Tab
   kannst du schließen.

Jede App meldest du einmal an. Das gilt für QOaroma, paf_note, paf_feedback,
paf_commodity, paf_coa, finance, paf_training, paf_freight und paf_force (Sales);
paf_supplychain folgt mit der nächsten Version. Warum der Umweg: Google erlaubt die
Anmeldung nicht innerhalb solcher Apps, und im Browser hast du deine Passwörter und
Passkeys.

Passiert nichts oder bleibt der Tab leer: im Tab der App im Menü **Planet A Foods →
Sign in via browser** (⌘L bzw. Strg+L) wählen und die App offen lassen, bis du
zurück bist.

## Tabs und was sich wo öffnet

- **Unsere Apps** öffnen in der App; Links, die sonst einen neuen Browser-Tab
  öffnen würden, öffnen einen neuen Tab in der App.
- **Personio, Moss und Ariadne** (früher Qwiki) öffnen in **deinem Browser**.
- **⌘T / Strg+T** neuer Tab (mit dem Hub), **⌘W / Strg+W** Tab schließen,
  **⌘1–9 / Strg+1–9** zu einem Tab springen.

## Neue Version installieren

Die App aktualisiert sich nicht selbst.

- **Mac:** neue `.dmg` laden, **Planet A Foods** in **Programme** ziehen,
  **Ersetzen** wählen, dann die Terminal-Zeile aus Schritt 3 noch einmal ausführen.
- **Windows:** neue `_x64-setup.exe` laden und ausführen. Sie ersetzt die alte Version.

Angemeldet bleibst du in der Regel.
