# Rozmiar bundla UI (D14)

Wyjście `npm run build` (Vite 6.4.6) w `ui/`. Próg ostrzeżenia Vite: 500 kB po minifikacji.

## Przed zmianami (main @ 8da8949, zadanie 2.1)

| Plik | raw | gzip |
|---|---|---|
| `dist/index.html` | 0.40 kB | 0.27 kB |
| `dist/assets/index-*.css` | 32.53 kB | 6.04 kB |
| `dist/assets/index-*.js` | 622.00 kB | 194.12 kB |

Uwaga: główny chunk przekraczał próg 500 kB już przed migracją (ostrzeżenie Vite „Some chunks are larger than 500 kB”).

## Po 2.1 (React 18.3.1 + zainstalowane, jeszcze nieużywane pakiety ADS)

| Plik | raw | gzip |
|---|---|---|
| `dist/assets/index-*.css` | 32.53 kB | 6.04 kB |
| `dist/assets/index-*.js` | 541.42 kB | 170.76 kB |

Spadek o ~81 kB wynika z zejścia z `react-dom` 19 na 18.3.1 (pakiety ADS nie są jeszcze importowane).

## Po fazie 2 (zadanie 2.9: motyw, powłoka page-layout + side-navigation, prymitywy i komponenty na ADS)

| Plik | raw | gzip |
|---|---|---|
| `dist/assets/index-*.css` | 126.69 kB | 25.10 kB |
| `dist/assets/index-*.js` | 1 268.82 kB | 371.90 kB |
| motywy `@atlaskit/tokens` (osobne chunki ładowane przez `setGlobalTheme`, m.in. `atlassian-light`, `atlassian-dark`) | ~0.4–28 kB każdy | ~0.2–3.6 kB |
| `custom-theme-*.js`, `delivery-*.js` (chunki dynamiczne z `@atlaskit/*`) | 68.53 / 55.08 kB | 16.86 / 19.65 kB |

Względem stanu sprzed zmian (622.00 kB / 194.12 kB gzip): główny chunk JS ×2.0 (+647 kB raw, +178 kB gzip), CSS +94 kB raw (css-reset i style komponentów ADS). Główny chunk nadal > 500 kB → `React.lazy` dla `Rules`, `Mappings`, `Actions` w zadaniu 3.5 (D14).

## Po fazie 3 (zadanie 3.5: scalenie G1–G4 + `React.lazy`)

Po scaleniu G1–G4, przed `React.lazy`:

| Plik | raw | gzip |
|---|---|---|
| `dist/assets/index-*.css` | 149.37 kB | 28.82 kB |
| `dist/assets/index-*.js` (główny) | 1 339.21 kB | 394.27 kB |
| `index-*.js` (refractor/highlighter z `CodeBlock`, już dynamiczny w `@atlaskit/code`) | 616.32 kB | 222.13 kB |

`React.lazy` tylko dla `Rules`, `Mappings`, `Actions` (D14): główny chunk 1 152.79 kB / 335.65 kB gzip — nadal > 500 kB.

Wybrany wariant: `React.lazy` dla wszystkich ekranów poza startowym feedem (`AlertFeed` zostaje w głównym chunku), `Suspense` z `Loading` (`Spinner`):

| Plik | raw | gzip |
|---|---|---|
| `dist/assets/index-*.css` | 103.42 kB | 21.25 kB |
| `dist/assets/index-*.js` (główny) | 969.69 kB | 279.48 kB |
| ekrany (`Rules`, `Mappings`, `Actions`, `AlertDetail`, `MockOutbox`, `People`, `Approvals`, `Runbooks`, `DevComponents`) | 2.8–36.4 kB każdy | 1.3–12.7 kB |
| wspólne chunki ekranów (`Markdown`, `public-api`, `ConditionView`, …) | do 120.83 kB | do 37.04 kB |
| `index-*.js` (refractor, dynamiczny) | 616.32 kB | 222.13 kB |

Względem stanu po fazie 2: główny chunk −299 kB raw (−92 kB gzip); względem sprzed zmian (622.00 / 194.12 kB) nadal ×1.56 raw / ×1.44 gzip.

Dlaczego główny chunk zostaje > 500 kB: nawet z leniwie ładowanymi wszystkimi ekranami (także `AlertFeed`) ma 947 kB — to sama powłoka i runtime ADS. Udział pakietów w głównym chunku (z source map, raw): `react-dom` 129 kB, `@atlaskit/react-select` 87 kB (`Select` w `SessionBar`), `@atlaskit/tokens` 58 kB, `@atlaskit/feature-gate-js-client` 58 kB + `@statsig/*` 75 kB + `@atlaskit/flag-client` 23 kB (flagi funkcji ciągnięte przez pakiety ADS, nieużywane przez aplikację), `@atlaskit/primitives` 55 kB, `@atlaskit/top-layer` 40 kB, `react-router` 38 kB, `@atlaskit/button` 31 kB, `@atlaskit/page-layout` 29 kB, `@atlaskit/dynamic-table` 28 kB (feed), `lucide-react` 22 kB. Zejście poniżej progu wymagałoby `manualChunks` (podział na pliki bez zmniejszenia tego, co ładuje się przy starcie) albo usunięcia powłoki ADS — poza zakresem; próg traktujemy jako znany wyjątek (D14).
