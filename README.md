# Hush

Hush gör ditt mikrofonljud bättre innan det når Teams, Zoom eller Slack. Den tar bort
bakgrundsljud, kan hålla andra röster i rummet utanför mötet, gör rösten tydligare och kan
ändra hur du låter. Allt sker lokalt på datorn. Inget ljud skickas någonstans och inget ljud
sparas.

## Så fungerar det

Hush sitter mellan din riktiga mikrofon och mötesprogrammet. Mötesprogrammet använder en
virtuell mikrofon, **Hush Microphone** (VB-CABLE på Windows, BlackHole på macOS), och Hush
skickar det bearbetade ljudet dit. Ljudet går igenom stegen i den här ordningen:

1. **Brusreducering**: DeepFilterNet, ett neuralt nätverk, tar bort fläktar, tangentbord,
   trafik och sorl.
2. **Röstgrind**: stänger mikrofonen när ingen pratar, så att svagt ljud i rummet inte hörs.
3. **Röstigenkänning**: känner igen vem som pratar och släpper bara igenom de röster du vill.
4. **Röstförbättring**: gör rösten tydligare och jämnare, som programvaran till
   studiomikrofoner.
5. **Röstfilter**: ändrar hur du låter, för den som vill.

Fördröjningen genom hela kedjan är cirka 50 ms, eller cirka 70 ms med ett röstfilter på. Det
märks inte i ett samtal. Hush använder ungefär 1 % av processorn.

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

## Ljud

Sidan **Ljud** är startsidan. Där väljer du mikrofon och ser ljudet före (In) och efter (Ut)
bearbetningen. Fördröjning och processorlast visas uppe till höger.

**Läge** styr vad Hush gör:

- **Av**: mikrofonen går orörd till mötet. Varken brusreducering, röstigenkänning eller
  röstförbättring används.
- **Dämpa bakgrundsljud**: brusreducering och röstgrind. Med en inspelad röstprofil styr
  röstigenkänningen dessutom vilka röster som hörs (se Röster).
- **Bara min röst**: bara din egen röst släpps igenom, alla andra röster stängs av. Kräver en
  inspelad röstprofil. Utan profil fungerar läget som Dämpa bakgrundsljud.

**Brusreducering** anger hur mycket bakgrundsljud som tas bort, upp till 45 dB. Höga värden
kan göra rösten något torrare.

**Röstgrind** stänger mikrofonen när ingen pratar nära den. Flytta reglaget åt höger (Bara nära
ljud) om röster längre bort i rummet hörs, åt vänster (Släpper in mer) om början eller slutet av
dina ord försvinner. Enklast är **Kalibrera**: var tyst i 3 sekunder medan Hush lyssnar på rummet,
prata sedan som vanligt i 5 sekunder. Hush lägger tröskeln strax under din röst och över
bakgrundsljudet. Röstgrinden sparas för varje mikrofon, så den följer med när du byter mellan
headset och datorns mikrofon.

Sidomenyn har tre knappar som fungerar från alla sidor:

- **Hush aktiv** pausar all bearbetning. Mikrofonljudet går då obehandlat till mötet.
- **Stäng av mikrofon** tystar helt. Snabbkommandot är Ctrl Alt M (Windows) eller ⌃⌥M (macOS)
  och fungerar även när mötesprogrammet är i fokus. Hush startar alltid med mikrofonen på.
- **Lyssna på dig själv** spelar upp exakt det mötet hör i datorns standardutgång, så att du kan
  höra brusreducering, röstförbättring och röstfilter. Använd hörlurar, annars når ljudet
  mikrofonen igen och det blir rundgång. Är standardutgången VB-CABLE vägrar Hush, eftersom ljudet
  då skulle gå rakt tillbaka in i mötet. Lyssningen stängs av när du minimerar Hush eller byter
  mikrofon.

Använder du en Bluetooth-mikrofon visar Hush en varning (den kan stängas av under Inställningar). Bluetooth-headset växlar till
telefonkvalitet så fort mikrofonen används, både för det du säger och det du hör. Datorns
inbyggda mikrofon eller headsetets USB-dongel ger bättre ljud.

## Röstförbättring

Under **Röstförbättring** på sidan Ljud gör du rösten tydligare och mer bekväm att lyssna på.
Det är samma sorts bearbetning som programvaran till studiomikrofoner gör. Den används i lägena
Dämpa bakgrundsljud och Bara min röst, påverkar bara den röst som släpps igenom och lägger inte
till någon märkbar fördröjning.

Ljudet går igenom fem steg:

1. **Lågklipp**: tar bort muller under ungefär 80 Hz, till exempel stötar i bordet, fläktbrus
   och luftstötar från p-ljud.
2. **Klang (EQ)**: tar ner det burkiga kring 300 Hz och lyfter tydligheten kring 3–5 kHz. Det
   är det som gör en röst lätt att förstå.
3. **De-esser**: dämpar vassa s- och sch-ljud, som headsetmikrofoner ofta överdriver. Den
   griper bara in när de vassa ljuden dominerar, så rösten behåller sin vanliga skärpa.
4. **Kompressor**: jämnar ut skillnaden mellan när du pratar tyst och högt. Det är det som gör
   en röst bekväm att lyssna på.
5. **Automatisk nivå och limiter**: håller din röst på en jämn nivå oavsett avstånd till
   mikrofonen och förhindrar att ljudet någonsin överstyr. Nivån justeras bara medan du pratar,
   så pauser förstärks inte till hörbart brus.

Välj en **profil**:

- **Naturlig**: bara lågklipp och jämn nivå. Rösten låter som vanligt.
- **Tydlig**: hela kedjan, anpassad för att höras i möten. Mindre burkigt, mer närvaro,
  dämpade s-ljud.
- **Varm**: mer botten och mjukare diskant, som en poddröst.

**Styrka** anger hur mycket klang, s-dämpning och utjämning som läggs på i Tydlig och Varm.
Profilen Naturlig har ingen styrka att ställa.

Stäng av mötesprogrammets egen brusreducering och automatiska mikrofonnivå när du använder
Hush, annars motverkar de bearbetningen. I Teams finns inställningarna under Inställningar >
Enheter, i Zoom under Inställningar > Ljud.

Röstförbättringen kan bara förbättra det mikrofonen fångar. En Bluetooth-mikrofon i
telefonkvalitet blir tydligare, men låter aldrig som en studiomikrofon.

## Röster

Röstigenkänningen håller isär rösterna i rummet, så att du kan bestämma vilka som hörs i mötet.
Den känner igen röster på ett röstavtryck: en sifferbeskrivning av hur rösten låter. Själva
ljudet spelas aldrig in och sparas aldrig.

**Din röstprofil kommer först.** Röstigenkänningen är helt avstängd tills du har spelat in din
röstprofil. Innan dess släpps alla röster igenom och inga nya röster upptäcks eller sparas. Utan
profilen kan Hush inte veta vilken röst som är din. Din egen röst skulle då hamna i listan som en
främmande röst, och du skulle kunna tysta dig själv.

Så spelar du in den:

1. Slå på Hush och välj läget **Dämpa bakgrundsljud**.
2. Gå till **Röster** och välj **Spela in**.
3. Läs texten högt i normal samtalston, i den miljö du brukar sitta i, tills mätaren är full.
   Det tar cirka 30 sekunder tal. Pauser räknas inte.

Därefter visar sidan en lista med alla röster Hush känner till:

- **Din röst** står först, känns alltid igen först och släpps alltid igenom. Profilen finslipas
  försiktigt med tiden när Hush är helt säker på att det är du, så att den följer med till nya
  headset och rum. Den kan aldrig glida långt från din inspelning, och spelar du in en ny profil
  börjar den om.
- Varje annan röst har **ett reglage: Hörs**. Av betyder att rösten hålls utanför mikrofonen.
  Valet gäller direkt och sparas, så det gäller också nästa gång rösten känns igen.
- Röster som har hörts de senaste 15 minuterna står överst, med en aktivitetslinje för de senaste
  20 sekunderna och hur säkert Hush känner igen dem (Träff). Övriga visar när de senast hördes.
- En ny röst dyker upp när någon har pratat i ungefär 2 sekunder. Den heter Röst 2, Röst 3 och så
  vidare tills du byter namn med pennan bredvid namnet.
- **Okända röster** längst ner styr tal som Hush ännu inte har känt igen: **Stäng av** (standard)
  eller **Släpp igenom**. Nya röster får samma val. Har du själv pratat den senaste minuten släpps
  okänt tal igenom tills det är identifierat, så att början av dina egna meningar aldrig klipps.
- I läget **Bara min röst** tystas alla röster utom din, oavsett reglagen. Sidan visar det, med
  en knapp för att byta tillbaka. Slår du på en röst i det läget byter Hush till Dämpa
  bakgrundsljud.
- Röster tas aldrig bort automatiskt. Ta bort en röst med papperskorgen och bekräfta med ett
  klick till.

Rösterna sparas krypterat (AES-256-GCM) på datorn. Nyckeln ligger i systemets nyckelring
(Windows Autentiseringshanteraren eller macOS Nyckelringar). Röstavtrycken lämnar aldrig datorn
och finns kvar när Hush uppdateras.

Hush lyssnar efter röster i lägena Dämpa bakgrundsljud och Bara min röst, även när andra röster
är avstängda. Igenkänningen sker innan någon tystas, så nya röster hamnar i listan även om de
aldrig hörs i mötet. Hush känner inte heller till möten: röster som hörs utanför möten, till
exempel en kollega i kontorslandskapet eller en TV, kan också hamna i listan. Pausa Hush med
**Hush aktiv** när du inte sitter i möte om du vill undvika det.

## Statistik

Sidan **Statistik** visar vem som har pratat mest, hur länge och hur starkt, räknat från
röstbiblioteket.

- Överst: total taltid, vem som pratar mest, vilken röst som är starkast och vem som har de
  längsta inläggen i snitt.
- En rad per röst, inklusive din egen: andel av taltiden, taltid, medelnivå, starkaste nivå,
  antal inlägg och genomsnittlig längd, och ett diagram över de senaste 14 dagarna. Sortera efter
  taltid, nivå eller antal inlägg.
- Bara tal som Hush har känt igen räknas, från ögonblicket rösten känns igen. Statistiken samlas
  därför bara in när röstprofilen är inspelad och Hush är i läget Dämpa bakgrundsljud eller Bara
  min röst. Den började räknas i version 0.5.0.
- Nivån visar hur starkt rösten når din mikrofon. Den beror mest på hur nära personen sitter, inte
  på hur högt hen faktiskt pratar.
- Dagarna sparas i 90 dagar, totalerna tills du nollställer med **Nollställ statistiken**.
  Statistiken sparas krypterad i röstbiblioteket, liksom röstavtrycken.

## Röstfilter

Under **Röstfilter** ändrar du hur du låter. Filtret läggs på sist, efter röstförbättringen,
och påverkar bara den röst som släpps igenom.

- **Tonhöjd** flyttar rösten upp eller ned, upp till 12 halvtoner (en oktav) åt varje håll.
- **Röstkaraktär** gör rösten mörkare eller ljusare, som en större eller mindre person.
  Tonhöjd och karaktär ställs var för sig. Högre tonhöjd med samma karaktär låter fortfarande
  som du, bara ljusare.
- **Klang**: **Naturlig**, **Robot** (monoton och metallisk) eller **Radio** (smalt band och
  lätt distorsion, som en liten högtalare).
- **Förval**: Filmtrailer, Rymdskurk, Sportkommentator, Robot, Troll, Radio 1985 och Helium.
  Förvalen ställer alla reglage på en gång. Drar du i ett reglage slås filtret på.
  **Återställ** stänger av filtret och nollar reglagen.

Filtret lägger till cirka 21 ms fördröjning medan det är på. Det är alltid avslaget när Hush
startar, så att du inte råkar börja ett möte med fel röst. Inställningarna finns kvar. Stora
ändringar, som Troll och Helium, låter lite metalliskt. Det är normalt för den här tekniken i
realtid.

Lyssna på resultatet med **Lyssna på dig själv** i sidomenyn.

## Inställningar och uppdateringar

Under **Inställningar**:

- **Starta med Windows** (Starta vid inloggning på macOS): Hush startar dold i aktivitetsfältet
  när du loggar in, så att den alltid är redo inför ett möte.
- **Stänga fönstret**: Hush fortsätter i aktivitetsfältet. Avsluta helt via ikonen där.
- **Varna för Bluetooth-mikrofon**: visar eller döljer varningen på sidan Ljud.
- **Felsökningslogg**: **Visa loggfiler** öppnar mappen där Hush skriver vad som händer (start,
  mikrofonbyten, fel). Skicka filen om något krånglar. Loggen innehåller inget ljud och inga
  röstavtryck.
- **Tema**: System, Ljust eller Mörkt.
- **Version** och **Sök efter uppdatering**.

Hush uppdaterar sig själv. Den söker efter nya versioner strax efter start och sedan var
sjätte timme. Du kan också söka direkt med **Sök efter uppdatering**. När en ny version finns
visas en rad överst. **Installera och starta om** laddar ner, kontrollerar signaturen och
installerar. Windows frågar efter administratörsbehörighet.

## Integritet

- Allt ljud bearbetas på datorn. Inget ljud lämnar den och inget ljud sparas.
- Röstigenkänningen sparar bara röstavtryck och statistik, krypterade, i röstbiblioteket.
- Hush ansluter bara till internet för att söka efter och ladda ner uppdateringar från GitHub.

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
node scripts/release.mjs 0.4.0 "Kort beskrivning av vad som är nytt"
```

Skriptet höjer versionsnumret, skapar en tagg och pushar. GitHub Actions bygger och signerar
båda plattformarna och publicerar en release. Installerade versioner av Hush hittar den inom
några timmar (eller direkt via Inställningar > Sök efter uppdatering) och frågar innan de installerar.

Uppdateringar signeras med nyckeln i `~/.tauri/hush-updater.key` (hemligheten
`TAURI_SIGNING_PRIVATE_KEY` i GitHub). Förlorad nyckel betyder att installerade versioner
inte kan uppdateras längre, så spara en kopia på ett säkert ställe.
