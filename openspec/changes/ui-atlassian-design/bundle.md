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
