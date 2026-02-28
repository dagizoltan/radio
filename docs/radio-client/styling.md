# Styling (style.css)

The visual design of the `radio-client` utilizes a clean, modern "radio aesthetic." It is implemented using plain CSS in `static/style.css` without any preprocessors (SASS/LESS) or utility frameworks (Tailwind).

## Custom Properties Palette

All colors and essential spacing values are defined as CSS Custom Properties (`--var`) on the `:root` pseudo-class to ensure consistency.

*   **Theme:** Light theme by default.
*   **Surfaces:** White card surfaces over a very light grey/blue background.
*   **Shadows:** Subtle drop shadows for depth.
*   **Accent:** A distinct accent blue for primary interactive elements.

## Layout and Components

### Player Card

The main UI component is a centered "Player Card".

*   It has rounded corners and a shadow.
*   It is divided into distinct sections vertically.

### Waveform Area

The top section of the card is the waveform visualizer.

*   **Background:** A dark, near-black color to contrast with the light theme of the rest of the card.
*   **Waveform Line:** The `<canvas>` element draws the audio waveform using the accent blue color.

### Play/Stop Button

*   A large, prominent, filled circular button.
*   Uses the accent blue color for the background.
*   Contains a white SVG icon (Play or Stop).
*   Includes hover and active states (e.g., slight scale transform or color darken).

### Live Indicator

*   A small circular dot next to the metadata.
*   **Live State:** Green color (`--color-success`).
*   **Offline State:** Grey color (`--color-offline`).
*   **Pulse Animation:** When live, applies a CSS `@keyframes` animation (`box-shadow` expansion and fade) to simulate a pulsing recording light.

### Latency Display

*   A small, subtle text readout indicating how many seconds behind the live edge the player is currently buffered.

### Volume Control

*   An `<input type="range">` slider.
*   Styled using pseudo-elements (`::-webkit-slider-thumb`, `::-webkit-slider-runnable-track`, `::-moz-range-thumb`, `::-moz-range-track`) to match the overall aesthetic (e.g., accent blue thumb, subtle grey track).

## Transitions

All state changes (hover effects, button toggles, color shifts) utilize smooth CSS `transition` properties.

*   **Duration:** Generally between `100ms` and `200ms` for a snappy but polished feel.
*   **Timing Function:** `ease-in-out` or standard `ease`.