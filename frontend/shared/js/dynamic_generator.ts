export type GeneratedElement = HTMLElement | SVGSVGElement;
export type ElementTarget = string | GeneratedElement;
type StyleValue = string | number;
type Styles = Record<string, StyleValue>;
type FormData = Record<string, string | number>;
type FormErrors = Record<string, string>;
type ValidationResult = FormErrors | null | undefined;
type EventDefinitions = Record<string, EventListener>
    | { type: string; callback: EventListener }[];

export interface ElementDefinition {
    type: string;
    target?: string;
    varId?: string;
    id?: string;
    text?: string | number;
    html?: string;
    classes?: string | string[];
    attributes?: Record<string, string | number | boolean | undefined>;
    properties?: Record<string, unknown>;
    style?: Styles;
    data?: Record<string, string | number | boolean>;
    events?: EventDefinitions;
    children?: ElementDefinition[];
    file?: string;
    sketch?: string;
    pathStyle?: Record<string, StyleValue | Styles>;
    label?: string;
    placeholder?: string;
    inputType?: string;
    value?: string | number;
    rows?: number;
    options?: ElementDefinition[];
}

export interface BoxOptions {
    isTemporary?: boolean;
    modal?: boolean;
    fullscreen?: boolean;
    width?: string;
    height?: string;
}

interface DialogOptions {
    title?: string;
    content?: ElementDefinition[];
    classes?: string[];
    isModal?: boolean;
    closeOnOverlayClick?: boolean;
    isTemporary?: boolean;
}

export interface PromptOptions {
    title: string;
    elements: ElementDefinition[];
    confirmText?: string;
    cancelText?: string;
    onConfirm?: (data: FormData) => void;
    onCancel?: () => void;
    onValidate?: ((data: FormData) => ValidationResult | Promise<ValidationResult>) | null;
    required?: boolean;
    isModal?: boolean;
}

const svgCache = new Map<string, string>();
const svgNamespace = "http://www.w3.org/2000/svg"; 
const validElementTypes = [ 
  'svg', 'code', 'li', 'strong', 'small', 'pre', 'textarea', 'nav', 'section', 
  'a', 'i', 'div', 'span', 'p', 'br', 'button', 'h1', 'h2', 'h3', 'input', 
  'img', 'label', "table", "thead", "tbody", "select", "option", "tr", "td", "th", "aside", "ul",
  "header", "main", "tfoot"
];

function resolveElement(elem: ElementTarget): GeneratedElement {
    if (elem instanceof HTMLElement || elem instanceof SVGSVGElement) return elem;
    const element = document.querySelector(elem);
    if (!(element instanceof HTMLElement || element instanceof SVGSVGElement)) {
        throw new Error(`Element target not found or unsupported: ${elem}`);
    }
    return element;
}

function compoundChildren(definition: ElementDefinition): ElementDefinition[] | null {
    const label: ElementDefinition = {
        type: "label",
        attributes: { for: definition.id },
        text: definition.label ?? "",
    };
    const control: ElementDefinition = {
        type: "",
        id: definition.id,
        events: definition.events,
    };
    switch (definition.type) {
        case "input-label":
            control.type = "input";
            control.attributes = {
                type: definition.inputType ?? "text", placeholder: definition.placeholder, value: definition.value,
            };
            break;
        case "select-label":
            control.type = "select";
            control.children = definition.options;
            break;
        case "textarea-label":
            control.type = "textarea";
            control.attributes = { rows: definition.rows, placeholder: definition.placeholder };
            control.properties = { value: definition.value ?? "" };
            break;
        default:
            return null;
    }
    control.attributes = { ...control.attributes, ...definition.attributes };
    return [label, control];
}

function applyStyles(element: GeneratedElement, styles: Styles): void {
    for (const [property, value] of Object.entries(styles)) {
        const name = property.startsWith("--")
            ? property
            : property.replace(/[A-Z]/g, letter => `-${letter.toLowerCase()}`);
        element.style.setProperty(name, String(value));
    }
}

const dynamicGenerator = (
    definition: ElementDefinition,
    location: ElementTarget = document.body,
): GeneratedElement => {
    const interpolatedElements = compoundChildren(definition);
    if (interpolatedElements) {
    
    // 1. Pass wrapper ID, style, and classes to the form-group container
    const groupContainer = dynamicGenerator({
        type: "div",
        id: definition.id ? `${definition.id}-group` : undefined, // Optional wrapper ID
        classes: ["form-group", ...(definition.classes ? [definition.classes].flat() : [])],
        style: definition.style // Now style: { display: "none" } works on the wrapper!
    }, location);

    // 2. Attach top-level events & attributes to the child input/select element
    interpolatedElements.forEach(childDef => dynamicGenerator(childDef, groupContainer));

    return groupContainer;
}

    if (!definition.type || typeof definition.type !== "string") throw new TypeError("Invalid element type");
    if (!validElementTypes.includes(definition.type)) throw new TypeError(`Unsupported element type: ${definition.type}`);

    const container = resolveElement(location);
    let item: GeneratedElement;

    switch( definition.type ) {
        case "svg":
            let svgContent: string;
            const hasSketch = definition.sketch !== undefined;

            if(hasSketch) {
                if(typeof definition.sketch !== "string") {
                    throw new TypeError("SVG sketch must be a string");
                }
                svgContent = definition.sketch;
            } else {
                if(!definition.file) throw new Error("SVG requires a file reference or sketch");

                if(!svgCache.has(definition.file)) {
                    const request = new XMLHttpRequest();
                    request.open("GET", definition.file, false);
                    request.send(null);

                    if(request.status === 200 && request.responseText.toLowerCase().includes("<?xml version")) {
                        svgCache.set(definition.file, request.responseText);
                    } else {
                        throw new Error(`Failed to request SVG file, ${definition.file}`);
                    }
                }

                const cached = svgCache.get(definition.file);
                if (cached === undefined) throw new Error(`SVG cache entry missing: ${definition.file}`);
                svgContent = cached;
            }

            const parser = new DOMParser();
            const svgDoc = parser.parseFromString(svgContent, "image/svg+xml");
            const loadedSvg = svgDoc.documentElement;
            if(loadedSvg.localName !== "svg") {
                throw new Error("Invalid SVG markup");
            }

            item = document.createElementNS(svgNamespace, definition.type);
            for(const attribute of loadedSvg.attributes) {
                item.setAttributeNS(attribute.namespaceURI, attribute.name, attribute.value);
            }

            const generalStyle = new Map<string, StyleValue>();
            const specificStyling = new Map<string, Styles>();

            if(typeof definition.pathStyle === "object") {
                for(const key in definition.pathStyle) {
                    if(typeof definition.pathStyle[key] === "object") {
                        specificStyling.set(key, definition.pathStyle[key]);
                    } else {
                        generalStyle.set(key, definition.pathStyle[key]);
                    }
                }
            }

            const allElements = loadedSvg.querySelectorAll("*");

            allElements.forEach(elem => {
                const finalStyle = Object.fromEntries(generalStyle);
                specificStyling.forEach((styles, selector) => {
                    if(elem.matches(selector)) Object.assign(finalStyle, styles);
                });
                if (elem instanceof SVGElement) {
                    for (const [prop, value] of Object.entries(finalStyle)) {
                        const name = prop.replace(/[A-Z]/g, letter => `-${letter.toLowerCase()}`);
                        elem.style.setProperty(name, String(value));
                    }
                }
            });

            while(loadedSvg.firstChild) item.appendChild(loadedSvg.firstChild);
            break;

        default:
            item = document.createElement(definition.type);
            if (definition.text !== undefined) item.innerText = String(definition.text);
            else if (definition.html !== undefined) item.innerHTML = definition.html;
    }

    try {
        if (Array.isArray(definition.classes)) {
            definition.classes.filter(c => c.length !== 0).forEach(c => item.classList.add(c));
        } else if (typeof definition.classes === "string") {
            item.classList.add(definition.classes);
        }
    } catch(e) {
        console.error(definition);
        throw e;
    }

    if (typeof definition.attributes === "object") {
        for (const attribute in definition.attributes) {
            // Strip out empty attributes resulting from optional template parameters
            if (definition.attributes[attribute] !== "" && definition.attributes[attribute] !== undefined) {
                item.setAttribute(attribute, String(definition.attributes[attribute]));
            }
        }
    }
    if(typeof definition.style === "object") applyStyles(item, definition.style);

    if(typeof definition.properties === "object")
        for(const property in definition.properties) {
            if (!Reflect.set(item, property, definition.properties[property])) {
                throw new Error(`Unable to set element property: ${property}`);
            }
        }

    if (definition.id !== undefined && typeof definition.id === "string") item.id = definition.id;

    if(definition.data !== undefined && typeof definition.data === "object") {
        const data = definition.data;
        Object.keys(data).forEach(tag => item.dataset[tag] = String(data[tag]));
    }

    if(Array.isArray(definition.events)) {
        definition.events.forEach(event => {
            if(event.type !== undefined && event.callback !== undefined) {
                item.addEventListener(event.type, event.callback);
            }
        });
    } else if(typeof definition.events === "object") {
        for(const event in definition.events) item.addEventListener(event, definition.events[event]);
    }

    if(definition.children !== undefined && Array.isArray(definition.children)) {
        definition.children.forEach(child => dynamicGenerator(child, item));
    }

    container.append(item);
    return item;
};

const generateDynamic = (definition: ElementDefinition[], location: ElementTarget = document.body): Record<string, GeneratedElement> => {
    if(!Array.isArray(definition)) throw new TypeError("Definition is not an array");
    const ids: Record<string, GeneratedElement> = {};
    const container = resolveElement(location);

    definition.forEach(item => {
        let target: GeneratedElement | undefined;
        if(item.target === undefined) {
            target = container;
        } else if(item.target.startsWith("@") && ids[item.target.slice(1)]) {
            target = ids[item.target.slice(1)];
        } else if(item.target !== undefined && item.target.charAt(0) !== "@") {
            target = resolveElement(item.target);
        }
        if(!target) {
            throw new Error(`Invalid target selector: ${item.target}`);
        }
        const elem = dynamicGenerator(item, target);
        if(item.varId !== undefined) ids[item.varId] = elem;
    });

    return ids;
};

const cleanContainer = (elem: ElementTarget): void => {
    const container = resolveElement(elem);
    while (container.firstChild) container.removeChild(container.firstChild);
};

class StackManager {
    stack: BaseDialog[] = [];
    readonly baseZIndex = 1000;
    readonly overlay: HTMLDivElement;

    constructor() {
        
        // Single shared backdrop overlay element driven by boxes.css
        this.overlay = document.createElement("div");
        this.overlay.classList.add("modal-overlay-backdrop");
        this.overlay.style.display = "none";
        this.overlay.addEventListener("click", () => this.handleBackdropClick());
        document.body.appendChild(this.overlay);
    }

    push(dialog: BaseDialog) {
        if (!this.stack.includes(dialog)) {
            this.stack.push(dialog);
            this.updateZIndices();
        }
    }

    remove(dialog: BaseDialog) {
        this.stack = this.stack.filter(d => d !== dialog);
        this.updateZIndices();
    }

    updateZIndices() {
        if (this.stack.length === 0) {
            this.overlay.style.display = "none";
            document.body.style.overflow = "";
            return;
        }

        document.body.style.overflow = "hidden";

        // Find highest modal requiring a backdrop
        let topModalIndex = this.stack.length - 1;
        while (topModalIndex >= 0 && !this.stack[topModalIndex].isModal) topModalIndex--;

        if (topModalIndex !== -1) {
            const overlayZ = this.baseZIndex + (topModalIndex * 10);
            this.overlay.style.display = "block";
            this.overlay.style.zIndex = String(overlayZ);
        } else {
            this.overlay.style.display = "none";
        }

        // Adjust each dialog's z-index sequentially
        this.stack.forEach((dialog, index) => {
            const dialogZ = this.baseZIndex + (index * 10) + 5;
            dialog.container.style.zIndex = String(dialogZ);
        });
    }

    handleBackdropClick() {
        const topDialog = this.stack[this.stack.length - 1];
        if (topDialog && topDialog.closeOnOverlayClick) {
            topDialog.close();
        }
    }
}

const DialogStack = new StackManager();

class BaseDialog {
    readonly isModal: boolean;
    readonly closeOnOverlayClick: boolean;
    readonly isTemporary: boolean;
    readonly contentDefinition: ElementDefinition[];
    container!: GeneratedElement;
    body!: GeneratedElement;
    header!: GeneratedElement;
    content: Record<string, GeneratedElement> = {};

    constructor({ 
        title = "", 
        content = [], 
        classes = [], 
        isModal = true, 
        closeOnOverlayClick = true,
        isTemporary = false 
    }: DialogOptions) {
        this.isModal = isModal;
        this.closeOnOverlayClick = closeOnOverlayClick;
        this.isTemporary = isTemporary;
        this.contentDefinition = content;

        this.buildWindow(title, classes);
    }

    buildWindow(title: string, extraClasses: string[]) {
        const dialogClasses = ["dialog-box", ...extraClasses];
        
        const items = generateDynamic([
            { type: "div", classes: dialogClasses, varId: "container" },
            { type: "div", classes: "modal-header", target: "@container", varId: "header" },
            { type: "h2", text: title, target: "@header" },
            { 
                type: "svg", 
                file: "icons/close.svg", 
                classes: "close-btn", 
                events: { click: () => this.close() }, 
                target: "@header" 
            },
            { type: "div", classes: "modal-content", target: "@container", varId: "body" }
        ]);

        this.container = items.container;
        this.body = items.body;
        this.header = items.header;

        this.content = generateDynamic(this.contentDefinition, this.body);
    }

    open() {
        this.container.style.display = "flex";
        DialogStack.push(this);
    }

    close() {
        this.container.style.display = "none";
        DialogStack.remove(this);
        if (this.isTemporary) {
            this.container.remove();
        }
    }
}

/**
 * Custom Promise-based dynamic form prompt modal.
 * Validators may return field errors synchronously or asynchronously.
 * Thrown validation errors reject the prompt; cancellation ignores pending results.
 */
const dynamicPrompt = ({
    title, 
    elements, 
    confirmText = "Save", 
    cancelText = "Cancel", 
    onConfirm, 
    onCancel, 
    onValidate = null,
    required = false,
    isModal = true
}: PromptOptions): Promise<FormData | null> => {
    return new Promise((resolve, reject) => {
        let promptDialog: BaseDialog;
        let validating = false;
        let settled = false;

        const clearErrors = (modalEl: GeneratedElement) => {
            modalEl.querySelectorAll('.modal-error-message').forEach(el => el.remove());
            modalEl.querySelectorAll('.invalid').forEach(el => el.classList.remove('invalid'));
            modalEl.querySelectorAll('[aria-invalid="true"]').forEach(el => el.removeAttribute('aria-invalid'));
        };

        const handleCancel = () => {
            if (settled) return;
            settled = true;
            promptDialog.close();
            if (onCancel) onCancel();
            resolve(null);
        };

        const handleConfirm = async () => {
            if (validating || settled) return;
            const modalEl = promptDialog.container;
            clearErrors(modalEl);

            const data: FormData = {};
            modalEl.querySelectorAll<HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement>('input[id], select[id], textarea[id]').forEach(el => {
                if (el.id) {
                    data[el.id] = el instanceof HTMLInputElement && el.type === 'number' ? el.valueAsNumber : el.value;
                }
            });

            if (onValidate) {
                const controls = Array.from(modalEl.querySelectorAll<HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement | HTMLButtonElement>('input, select, textarea, button'));
                const disabledStates = controls.map(control => control.disabled);
                const confirmButton = promptDialog.content.confirmButton;
                const validationStatus = promptDialog.content.validationStatus;
                validating = true;
                modalEl.setAttribute('aria-busy', 'true');
                controls.forEach(control => {
                    if (control !== promptDialog.content.cancelButton) control.disabled = true;
                });
                confirmButton.textContent = 'Validating...';
                validationStatus.removeAttribute('hidden');

                let errors: ValidationResult;
                try {
                    errors = await onValidate(data);
                } catch (error) {
                    if (!settled) {
                        settled = true;
                        promptDialog.close();
                        reject(error);
                    }
                    return;
                } finally {
                    validating = false;
                    modalEl.removeAttribute('aria-busy');
                    controls.forEach((control, index) => control.disabled = disabledStates[index]);
                    confirmButton.textContent = confirmText;
                    validationStatus.setAttribute('hidden', '');
                }
                if (settled) return;
                if (errors && Object.keys(errors).length > 0) {
                    Object.keys(errors).forEach(inputId => {
                        const errorTarget = modalEl.querySelector(`#${CSS.escape(inputId)}`);
                        if (errorTarget) {
                            const fieldGroup = errorTarget.closest('.form-group') || errorTarget.parentElement;
                            if (!(fieldGroup instanceof HTMLElement)) {
                                throw new Error(`Missing form group for: ${inputId}`);
                            }
                            const controls = errorTarget.matches('input, select, textarea')
                                ? [errorTarget]
                                : Array.from(fieldGroup.querySelectorAll('input, select, textarea'));

                            controls.forEach(control => {
                                control.classList.add('invalid');
                                control.setAttribute('aria-invalid', 'true');
                            });

                            const errorMessage = dynamicGenerator({
                                type: 'span',
                                classes: 'modal-error-message',
                                text: errors[inputId]
                            }, fieldGroup);
                            errorMessage.setAttribute('role', 'alert');
                        }
                    });
                    return;
                }
            }

            if (onConfirm) onConfirm(data);
            settled = true;
            resolve(data);
            promptDialog.close();
        };

        // Prepare elements target
        const contentElements = elements.map(el => ({
            ...el,
            target: el.target ?? '@modalContent'
        }));

        const promptDefinition: ElementDefinition[] = [
            { type: "div", classes: "modal-body-wrapper", varId: "modalContent" },
            { type: "div", classes: "modal-footer", varId: "modalFooter" },
            {
                type: "span",
                classes: "modal-validation-status",
                varId: "validationStatus",
                text: "Validating...",
                attributes: { role: "status", hidden: true },
                target: "@modalFooter"
            },
            ...(!required ? [{
                type: "button",
                classes: ["btn", "btn-secondary"],
                varId: "cancelButton",
                text: cancelText,
                target: "@modalFooter",
                events: { click: handleCancel }
            }] : []),
            {
                type: "button",
                classes: ["btn", "btn-primary"],
                varId: "confirmButton",
                text: confirmText,
                target: "@modalFooter",
                events: { click: handleConfirm }
            },
            ...contentElements
        ];

        promptDialog = new BaseDialog({
            title,
            content: promptDefinition,
            classes: ["modal-box"],
            isModal,
            isTemporary: true,
            closeOnOverlayClick: !required
        });

        if (required) {
            promptDialog.container.querySelector('.close-btn')?.remove();
        }

        const closeDialog = promptDialog.close.bind(promptDialog);
        promptDialog.close = () => {
            if (!settled) {
                handleCancel();
                return;
            }
            closeDialog();
        };

        promptDialog.open();
    });
};

/**
 * HoverBox instance logic
 */
const boxes: Record<string, Box> = {};

class Box extends BaseDialog {
    readonly options: BoxOptions;

    constructor(title: string, content: ElementDefinition[], options: BoxOptions = {}) {
        const isTemporary = options.isTemporary ?? false;
        const isModal = options.modal ?? true;

        super({
            title,
            content,
            classes: ["box-container"],
            isModal,
            isTemporary
        });

        this.options = options;
        this.setStyle();
        if (this.isTemporary) this.openBox();
    }

    setStyle() {
        this.options.width = this.options.width || "75%";
        this.options.height = this.options.height || "50%";

        if (this.options.fullscreen) {
            this.container.classList.add("fullscreen");
        } else {
            this.container.classList.remove("fullscreen");
            // Set dynamic dimensions without overriding CSS centering rules
            this.container.style.width = this.options.width;
            this.container.style.height = this.options.height;
        }
    }

    refreshContent(content: ElementDefinition[]) {
        cleanContainer(this.body);
        this.content = generateDynamic(content, this.body);
    }

    openBox() { this.open(); }
    closeBox() { this.close(); }
}

function addHoverBox(id: string | null, title: string, content: ElementDefinition[], options: BoxOptions = {}): Box {
    if (id === null) {
        if (options.isTemporary === undefined) options.isTemporary = true;
        let tempBox = new Box(title, content, options);
        return tempBox;
    } else {
        boxes[id] = new Box(title, content, options);
        return boxes[id];
    }
}

export {
    addHoverBox,
    boxes,
    dynamicPrompt,
    dynamicGenerator,
    generateDynamic,
    dynamicGenerator as dynamic_generator,
    generateDynamic as generate_dynamic,
};