# Hush

Tar bort bakgrundsljud från mikrofonen innan ljudet når Teams, Zoom eller Slack.
Ljudet bearbetas lokalt på datorn med DeepFilterNet och skickas till en virtuell mikrofon.

## Installera

### Windows

1. Kör `Hush_<version>_x64-setup.exe`. Appen är inte signerad ännu, så Windows visar
   "Windows skyddade datorn": välj **Mer info** och sedan **Kör ändå**.
2. Starta Hush. Saknas VB-CABLE visar Hush en knapp till nedladdningen. Packa upp filen,
   kör `VBCABLE_Setup_x64.exe` som administratör och starta om datorn.
3. Välj **Byt namn** i Hush så att mikrofonen heter Hush Microphone.
4. Välj **Hush Microphone** som mikrofon i mötesprogrammet.

### macOS

1. Öppna `Hush_<version>_universal.dmg` och dra Hush till Program.
2. Appen är inte notariserad ännu. Första gången: högerklicka på Hush och välj **Öppna**,
   eller tillåt den under Systeminställningar > Integritet och säkerhet.
3. Installera [BlackHole 2ch](https://existential.audio/blackhole/) (gratis). Hush visar en
   knapp till nedladdningen om den saknas.
4. Tillåt mikrofonen när macOS frågar.
5. Välj **BlackHole 2ch** som mikrofon i mötesprogrammet.

## Röster

Under **Röster** spelar du in din röstprofil (cirka 30 sekunder tal). Därefter känner Hush igen
din röst och andra röster i rummet. Nya röster sparas i röstbiblioteket tills du tar bort dem.
Stäng av en röst för att hålla den utanför mikrofonen, eller välj läget **Bara min röst**.
Röstprofilerna sparas krypterat (AES-256, nyckeln i systemets nyckelring) och lämnar aldrig datorn.

Snabbkommando för att stänga av mikrofonen: Ctrl Alt M (Windows), ⌃⌥M (macOS).

## Röstfilter

Under **Röstfilter** ändrar du hur du låter: tonhöjd, röstkaraktär och klang (naturlig, robot
eller radio), eller ett av förvalen. Filtret läggs på efter brusreduceringen och påverkar bara
din egen röst. Det lägger till cirka 21 ms fördröjning medan det är på och är alltid avslaget
när Hush startar. **Lyssna på dig själv** spelar upp det mötet hör i datorns
standardutgång. Använd hörlurar, annars når ljudet mikrofonen igen.

## Utveckling

```
npm install
npx tauri dev
cargo test --lib --manifest-path src-tauri/Cargo.toml
```

Installationsfiler byggs i GitHub Actions för båda plattformarna (se fliken Actions),
eller lokalt med `npx tauri build`.

## Tredjepart

- DeepFilterNet (MIT/Apache-2.0), lågfördröjningsmodellen DeepFilterNet3 LL.
- Röstigenkänning: [WeSpeaker](https://github.com/wenet-e2e/wespeaker) CAM++ (large margin),
  tränad på VoxCeleb, licens CC BY 4.0. ONNX-version från sherpa-onnx.
- VB-CABLE (donationware, VB-Audio Software) och BlackHole (GPL-3.0, Existential Audio)
  ingår inte i Hush. Användaren installerar dem själv.
- Typsnitt: Instrument Sans och IBM Plex Mono (SIL Open Font License 1.1, se `src/fonts`).

## Släppa en ny version

```
node scripts/release.mjs 0.2.1 "Kort beskrivning av vad som är nytt"
```

Skriptet höjer versionsnumret, skapar en tagg och pushar. GitHub Actions bygger och signerar
båda plattformarna och publicerar en release. Installerade versioner av Hush hittar den inom
några timmar (eller direkt via Inställningar > Sök efter uppdatering) och frågar innan de installerar.

Uppdateringar signeras med nyckeln i `~/.tauri/hush-updater.key` (hemligheten
`TAURI_SIGNING_PRIVATE_KEY` i GitHub). Förlorad nyckel betyder att installerade versioner
inte kan uppdateras längre, så spara en kopia på ett säkert ställe.
