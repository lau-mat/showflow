// Compound form components for the time-related inputs, registered next to the generator's own
// `input-label` / `select-label` / `textarea-label`:
//
//   time-label      hours : minutes : seconds dials with presets. config.mode "clock" (time of day)
//                   or "duration"; value and result are seconds.
//   datetime-label  a month calendar plus a clock dial; value and result are epoch seconds.
//   choice-label    a segmented control for a short list of options (config.options).
//
// Each keeps its value in one hidden `input[id]`, so `dynamicPrompt` collects and validates it
// like any other field (an error on the id outlines the whole component). Changes fire a
// `change` event on that input, and `events` given to the component are attached to it.
// Importing this file registers the components.

import { generateDynamic, registerCompound, type ElementDefinition } from "/shared/js/dynamic_generator";
import {
    clampSeconds, composeDateTime, dateKey, decomposeDateTime, formatDateTime, joinSeconds, monthGrid, nextFullHour,
    pad, parseDateKey, parseSegment, segmentMax, splitSeconds, stepSegment, type TimeMode
} from "/shared/js/time_math";

export interface TimePreset {
    label: string;
    seconds: number;
    /** Replace the value instead of adding to it. */
    set?: boolean;
}

export const DEFAULT_DURATION_PRESETS: TimePreset[] = [
    { label: "+30s", seconds: 30 },
    { label: "+1m", seconds: 60 },
    { label: "+5m", seconds: 300 },
    { label: "+15m", seconds: 900 },
    { label: "+1h", seconds: 3600 },
    { label: "Reset", seconds: 0, set: true }
];

const SEGMENT_NAMES = ["hours", "minutes", "seconds"];

const chevron = (direction: "up" | "down" | "left" | "right"): ElementDefinition =>
    ({ type: "svg", file: "icons/chevron.svg", classes: ["chev", `chev-${direction}`] });

function hiddenValue(definition: ElementDefinition, value: string | number): ElementDefinition {
    return {
        type: "input", id: definition.id, classes: "time-value",
        attributes: { type: "hidden", value: String(value) }, events: definition.events
    };
}

/** Sets a component's hidden value and tells listeners. */
function publish(hidden: HTMLInputElement, value: number | string): void {
    hidden.value = String(value);
    hidden.dispatchEvent(new Event("change", { bubbles: true }));
}

// ---------------------------------------------------------------------------------------------
// The dial: hours : minutes : seconds
// ---------------------------------------------------------------------------------------------

interface TimeFieldOptions {
    mode: TimeMode;
    seconds: number;
    presets: TimePreset[];
    /** Called with the new total whenever it changes. */
    onChange: (total: number, field: HTMLElement) => void;
}

const fieldOf = (element: Element): HTMLElement => element.closest<HTMLElement>(".time-field")!;
const segmentInputs = (field: HTMLElement): HTMLInputElement[] => [...field.querySelectorAll<HTMLInputElement>(".seg-input")];

function readTotal(field: HTMLElement): number {
    const mode = field.dataset.mode as TimeMode;
    const [h, m, s] = segmentInputs(field).map((input, index) => parseSegment(input.value, segmentMax(mode, index)));
    return joinSeconds(h, m, s);
}

function writeTotal(field: HTMLElement, total: number): void {
    const { h, m, s } = splitSeconds(total);
    segmentInputs(field).forEach((input, index) => { input.value = pad([h, m, s][index]); });
}

function timeFieldDefinition(options: TimeFieldOptions): ElementDefinition {
    const { mode } = options;
    const commit = (field: HTMLElement, total: number = readTotal(field)): void => {
        const clamped = clampSeconds(total, mode);
        writeTotal(field, clamped);
        options.onChange(clamped, field);
    };

    const segment = (index: number): ElementDefinition => {
        const max = segmentMax(mode, index);
        // Two digits fill a dial and move on (so 02:30:00 types as 023000); the hours of a long
        // duration can reach three digits, which the arrows and presets cover.
        const digits = String(max).length;
        const step = (element: Element, delta: number): void => {
            const field = fieldOf(element);
            const input = segmentInputs(field)[index];
            input.value = pad(stepSegment(parseSegment(input.value, max), delta, max));
            commit(field);
        };
        const focusNext = (input: HTMLInputElement): void => {
            const next = segmentInputs(fieldOf(input))[index + 1];
            next?.focus();
        };
        const { h, m, s } = splitSeconds(options.seconds);
        const label = SEGMENT_NAMES[index];
        const arrow = (direction: "up" | "down"): ElementDefinition => ({
            type: "button", classes: "seg-btn", attributes: { type: "button", tabindex: -1, "aria-label": `${direction === "up" ? "More" : "Fewer"} ${label}` },
            events: { click: (event: Event) => step(event.currentTarget as Element, direction === "up" ? 1 : -1) },
            children: [chevron(direction)]
        });
        return {
            type: "div", classes: "time-seg", children: [
                arrow("up"),
                {
                    type: "input", classes: "seg-input",
                    attributes: { type: "text", inputmode: "numeric", autocomplete: "off", maxlength: digits, "aria-label": label, value: pad([h, m, s][index]) },
                    events: {
                        keydown: (event: Event) => {
                            const key = (event as KeyboardEvent);
                            const input = key.currentTarget as HTMLInputElement;
                            if (key.key === "ArrowUp" || key.key === "ArrowDown") {
                                key.preventDefault();
                                step(input, (key.key === "ArrowUp" ? 1 : -1) * (key.shiftKey ? 10 : 1));
                            } else if ([":", ".", ",", " "].includes(key.key)) {
                                key.preventDefault();
                                focusNext(input);
                            }
                        },
                        wheel: (event: Event) => {
                            const wheel = event as WheelEvent;
                            const input = wheel.currentTarget as HTMLInputElement;
                            // Only a focused dial reacts, so scrolling past the dialog never changes a time.
                            if (document.activeElement !== input) return;
                            wheel.preventDefault();
                            step(input, wheel.deltaY < 0 ? 1 : -1);
                        },
                        focus: (event: Event) => { (event.currentTarget as HTMLInputElement).select(); },
                        input: (event: Event) => {
                            const input = event.currentTarget as HTMLInputElement;
                            input.value = input.value.replace(/\D/g, "");
                            if (Number(input.value) > max) input.value = String(max);
                            options.onChange(clampSeconds(readTotal(fieldOf(input)), mode), fieldOf(input));
                            // A full number moves on to the next dial, so a time can be typed in one go.
                            if (input.value.length >= 2 || Number(input.value) * 10 > max) focusNext(input);
                        },
                        blur: (event: Event) => commit(fieldOf(event.currentTarget as Element))
                    }
                },
                arrow("down")
            ]
        };
    };

    const colon: ElementDefinition = { type: "span", classes: "time-colon", text: ":" };
    const presets: ElementDefinition[] = options.presets.length === 0 ? [] : [{
        type: "div", classes: "time-presets", children: options.presets.map(preset => ({
            type: "button", classes: "chip", text: preset.label, attributes: { type: "button", tabindex: -1 },
            events: {
                click: (event: Event) => {
                    const field = fieldOf(event.currentTarget as Element);
                    commit(field, preset.set ? preset.seconds : readTotal(field) + preset.seconds);
                }
            }
        }))
    }];

    return {
        type: "div", classes: "time-field", data: { mode },
        children: [
            { type: "div", classes: "time-segs", children: [segment(0), colon, segment(1), colon, segment(2)] },
            ...presets
        ]
    };
}

// ---------------------------------------------------------------------------------------------
// time-label
// ---------------------------------------------------------------------------------------------

registerCompound("time-label", definition => {
    const config = definition.config ?? {};
    const mode: TimeMode = config.mode === "clock" ? "clock" : "duration";
    const seconds = clampSeconds(Number(definition.value ?? 0), mode);
    const presets = (config.presets as TimePreset[] | undefined) ?? (mode === "duration" ? DEFAULT_DURATION_PRESETS : []);
    return [
        { type: "label", text: definition.label ?? "" },
        hiddenValue(definition, seconds),
        timeFieldDefinition({
            mode, seconds, presets,
            onChange: (total, field) => {
                const hidden = field.closest(".form-group")!.querySelector<HTMLInputElement>(".time-value")!;
                if (hidden.value !== String(total)) publish(hidden, total);
            }
        })
    ];
});

// ---------------------------------------------------------------------------------------------
// datetime-label
// ---------------------------------------------------------------------------------------------

const monthTitle = (year: number, month: number): string =>
    new Date(year, month, 1).toLocaleDateString(undefined, { month: "long", year: "numeric" });

/** Monday-first narrow weekday names in the user's language (1 Jan 2024 was a Monday). */
const weekdayNames = (): string[] =>
    Array.from({ length: 7 }, (_, i) => new Date(2024, 0, 1 + i).toLocaleDateString(undefined, { weekday: "narrow" }));

const pickerOf = (element: Element): HTMLElement => element.closest<HTMLElement>(".datetime-picker")!;
const pickerValue = (root: HTMLElement): HTMLInputElement => root.parentElement!.querySelector<HTMLInputElement>(".time-value")!;

function dayDefinitions(year: number, month: number, selected: string): ElementDefinition[] {
    const today = dateKey(new Date());
    return monthGrid(year, month).map(cell => ({
        type: "button",
        classes: ["cal-day", cell.inMonth ? "" : "is-outside", cell.key === selected ? "is-selected" : "", cell.key === today ? "is-today" : ""],
        text: cell.day,
        attributes: { type: "button", "aria-pressed": cell.key === selected },
        data: { date: cell.key },
        events: { click: (event: Event) => pickDate(pickerOf(event.currentTarget as Element), cell.key) }
    }));
}

function renderCalendar(root: HTMLElement): void {
    const year = Number(root.dataset.viewYear);
    const month = Number(root.dataset.viewMonth);
    const selected = decomposeDateTime(Number(pickerValue(root).value)).date;
    root.querySelector(".cal-title")!.textContent = monthTitle(year, month);
    const grid = root.querySelector<HTMLElement>(".cal-grid")!;
    grid.replaceChildren();
    generateDynamic(dayDefinitions(year, month, selected), grid);
}

function showMonth(root: HTMLElement, year: number, month: number): void {
    const first = new Date(year, month, 1);
    root.dataset.viewYear = String(first.getFullYear());
    root.dataset.viewMonth = String(first.getMonth());
    renderCalendar(root);
}

function updateSummary(root: HTMLElement): void {
    root.querySelector(".datetime-summary")!.textContent = formatDateTime(Number(pickerValue(root).value));
}

function pickDate(root: HTMLElement, key: string): void {
    const hidden = pickerValue(root);
    const { secondsOfDay } = decomposeDateTime(Number(hidden.value));
    hidden.value = String(composeDateTime(key, secondsOfDay));
    const { year, month } = parseDateKey(key);
    showMonth(root, year, month);
    updateSummary(root);
    hidden.dispatchEvent(new Event("change", { bubbles: true }));
}

registerCompound("datetime-label", definition => {
    const epoch = Number(definition.value) > 0 ? Number(definition.value) : nextFullHour();
    const { date, secondsOfDay } = decomposeDateTime(epoch);
    const { year, month } = parseDateKey(date);

    const nav = (delta: number, label: string): ElementDefinition => ({
        type: "button", classes: "cal-nav", attributes: { type: "button", "aria-label": label },
        events: {
            click: (event: Event) => {
                const root = pickerOf(event.currentTarget as Element);
                showMonth(root, Number(root.dataset.viewYear), Number(root.dataset.viewMonth) + delta);
            }
        },
        children: [chevron(delta < 0 ? "left" : "right")]
    });

    return [
        { type: "label", text: definition.label ?? "" },
        hiddenValue(definition, epoch),
        {
            type: "div", classes: "datetime-picker", data: { viewYear: year, viewMonth: month },
            children: [
                {
                    type: "div", classes: "cal", children: [
                        {
                            type: "div", classes: "cal-head", children: [
                                nav(-1, "Previous month"),
                                { type: "span", classes: "cal-title", text: monthTitle(year, month) },
                                {
                                    type: "button", classes: "chip", text: "Today", attributes: { type: "button" },
                                    events: { click: (event: Event) => pickDate(pickerOf(event.currentTarget as Element), dateKey(new Date())) }
                                },
                                nav(1, "Next month")
                            ]
                        },
                        { type: "div", classes: "cal-weekdays", children: weekdayNames().map(name => ({ type: "span", text: name })) },
                        { type: "div", classes: "cal-grid", children: dayDefinitions(year, month, date) }
                    ]
                },
                timeFieldDefinition({
                    mode: "clock", seconds: secondsOfDay, presets: [],
                    onChange: (total, field) => {
                        const root = pickerOf(field);
                        const hidden = pickerValue(root);
                        const next = composeDateTime(decomposeDateTime(Number(hidden.value)).date, total);
                        if (hidden.value !== String(next)) {
                            hidden.value = String(next);
                            updateSummary(root);
                            hidden.dispatchEvent(new Event("change", { bubbles: true }));
                        }
                    }
                }),
                { type: "span", classes: "datetime-summary", text: formatDateTime(epoch) }
            ]
        }
    ];
});

// ---------------------------------------------------------------------------------------------
// choice-label
// ---------------------------------------------------------------------------------------------

export interface Choice {
    value: string;
    label: string;
}

registerCompound("choice-label", definition => {
    const choices = (definition.config?.options as Choice[] | undefined) ?? [];
    const current = String(definition.value ?? choices[0]?.value ?? "");
    const choose = (button: Element, value: string): void => {
        const group = button.closest<HTMLElement>(".choice-group")!;
        group.querySelectorAll<HTMLElement>(".choice-btn").forEach(other => {
            const active = other.dataset.value === value;
            other.classList.toggle("is-active", active);
            other.setAttribute("aria-checked", String(active));
        });
        const hidden = group.parentElement!.querySelector<HTMLInputElement>(".time-value")!;
        if (hidden.value !== value) publish(hidden, value);
    };
    return [
        { type: "label", text: definition.label ?? "" },
        hiddenValue(definition, current),
        {
            type: "div", classes: "choice-group", attributes: { role: "radiogroup" },
            children: choices.map(choice => ({
                type: "button", classes: ["choice-btn", choice.value === current ? "is-active" : ""], text: choice.label,
                attributes: { type: "button", role: "radio", "aria-checked": choice.value === current },
                data: { value: choice.value },
                events: { click: (event: Event) => choose(event.currentTarget as Element, choice.value) }
            }))
        }
    ];
});
