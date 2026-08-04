const svgCache = new Map(); 
const svgNamespace = "http://www.w3.org/2000/svg"; 
const validElementTypes = [ 
  'svg', 'code', 'li', 'strong', 'small', 'pre', 'textarea', 'nav', 'section', 
  'a', 'i', 'div', 'span', 'p', 'br', 'button', 'h1', 'h2', 'h3', 'input', 
  'img', 'label', "table", "thead", "tbody", "select", "option", "tr", "td", "th", "aside", "ul"
];

const compoundComponents = {
    "input-label": [
        { type: "label", attributes: { for: "&id" }, text: "&label" },
        { type: "input", attributes: { type: "text", placeholder: "&placeholder", value: "&value" }, id: "&id" }
    ],
    "select-label": [
        { type: "label", attributes: { for: "&id" }, text: "&label" },
        { type: "select", id: "&id", children: "&options" }
    ],
    "textarea-label": [
        { type: "label", attributes: { for: "&id" }, text: "&label" },
        { type: "textarea", attributes: { rows: "&rows", placeholder: "&placeholder" }, properties: {value: "&value"}, id: "&id" }
    ]
};

function resolveElement(elem) {
    if (elem instanceof HTMLElement) return elem;
    return document.querySelector(elem);
}

function interpolateTemplate(template, props) {
    if (typeof template === "string") {
        if (template.startsWith("&")) {
            const key = template.slice(1);
            return props[key] !== undefined ? props[key] : "";
        }
        return template;
    }

    if (Array.isArray(template)) {
        return template.map(item => interpolateTemplate(item, props));
    }

    if (typeof template === "object" && template !== null) {
        const result = {};
        for (const [key, value] of Object.entries(template)) {
            result[key] = interpolateTemplate(value, props);
        }
        return result;
    }

    return template;
}

const dynamicGenerator = ( definition, location = document.body ) => {
    if (definition.type && compoundComponents[definition.type]) {
    const template = compoundComponents[definition.type];
    const interpolatedElements = interpolateTemplate(template, definition);
    
    // 1. Pass wrapper ID, style, and classes to the form-group container
    const groupContainer = dynamicGenerator({
        type: "div",
        id: definition.id ? `${definition.id}-group` : undefined, // Optional wrapper ID
        classes: ["form-group", ...(definition.classes ? [definition.classes].flat() : [])],
        style: definition.style // Now style: { display: "none" } works on the wrapper!
    }, location);

    // 2. Attach top-level events & attributes to the child input/select element
    interpolatedElements.forEach(childDef => {
        if (["select", "input", "textarea"].includes(childDef.type)) {
            if (definition.events) childDef.events = definition.events;
            if (definition.attributes) {
                childDef.attributes = { ...childDef.attributes, ...definition.attributes };
            }
        }
        dynamicGenerator(childDef, groupContainer);
    });

    return groupContainer;
}

    if (!definition.type || typeof definition.type !== "string") return console.error("Invalid element type:", definition);
    if (!validElementTypes.includes(definition.type)) return console.error("Unsupported element type:", definition.type);

    location = resolveElement(location);
    let item = null;

    switch( definition.type ) {
        case "svg":
            if(!definition.file) return console.error("trying to add svg without file reference");

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

            const svgContent = svgCache.get(definition.file);
            item = document.createElementNS(svgNamespace, definition.type);

            const parser = new DOMParser();
            const svgDoc = parser.parseFromString(svgContent, "image/svg+xml");
            const loadedSvg = svgDoc.documentElement;

            if(loadedSvg.getAttribute("viewBox")) {
                item.setAttribute("viewBox", loadedSvg.getAttribute("viewBox"));
            }

            let generalStyle = new Map();
            let specificStyling = new Map();

            if(typeof definition.pathStyle === "object") {
                for(const key in definition.pathStyle) {
                    if(typeof definition.pathStyle[key] === "object") {
                        specificStyling.set(key, definition.pathStyle[key]);
                    } else {
                        generalStyle.set(key, definition.pathStyle[key]);
                    }
                }
            }

            const toKebabCase = str => str.replace(/[A-Z]/g, letter => `-${letter.toLowerCase()}`);
            const allElements = loadedSvg.querySelectorAll("*");

            allElements.forEach(elem => {
                const finalStyle = Object.fromEntries(generalStyle);
                specificStyling.forEach((styles, selector) => {
                    if(elem.matches(selector)) Object.assign(finalStyle, styles);
                });
                for(const prop in finalStyle) elem.style[toKebabCase(prop)] = finalStyle[prop];
            });

            while(loadedSvg.firstChild) item.appendChild(loadedSvg.firstChild);
            break;

        default:
            item = document.createElement(definition.type);
            if (definition.text !== undefined) item.innerText = definition.text;
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
        throw Error(e);
    }

    if (typeof definition.attributes === "object") {
        for (const attribute in definition.attributes) {
            // Strip out empty attributes resulting from optional template parameters
            if (definition.attributes[attribute] !== "" && definition.attributes[attribute] !== undefined) {
                item.setAttribute(attribute, definition.attributes[attribute]);
            }
        }
    }
    if(typeof definition.style === "object")
        for(const style in definition.style) item.style[style] = definition.style[style];

    if(typeof definition.properties === "object")
        for(const property in definition.properties) item[property] = definition.properties[property];

    if (definition.id !== undefined && typeof definition.id === "string") item.id = definition.id;

    if(definition.data !== undefined && typeof definition.data === "object") {
        Object.keys(definition.data).forEach(tag => item.dataset[tag] = definition.data[tag]);
    }

    if(Array.isArray(definition.events)) {
        definition.events.forEach(event => {
            if(event.type !== undefined && event.callback !== undefined) {
                item.addEventListener(event.type, event.callback.bind(this));
            }
        });
    } else if(typeof definition.events === "object") {
        for(const event in definition.events) item.addEventListener(event, definition.events[event].bind(this));
    }

    if(definition.children !== undefined && Array.isArray(definition.children)) {
        definition.children.forEach(child => dynamicGenerator(child, item));
    }

    location.append(item);
    return item;
};

const generateDynamic = (definition, location = document.body) => {
    if(!Array.isArray(definition)) return console.error("Definition is not an array");
    let ids = {};
    location = resolveElement(location);

    definition.forEach(item => {
        let target;
        if(item.target === undefined) {
            target = location;
        } else if(item.target.startsWith("@") && ids[item.target.slice(1)]) {
            target = ids[item.target.slice(1)];
        } else if(item.target !== undefined && item.target.charAt(0) !== "@") {
            target = document.querySelector(item.target);
        }
        if(!target) {
            console.error("Invalid target selector:", item.target, item);
            return;
        }
        const elem = dynamicGenerator(item, target);
        if(item.varId !== undefined) ids[item.varId] = elem;
    });

    return ids;
};

const cleanContainer = elem => {
    const container = resolveElement(elem);
    while (container.firstChild) container.removeChild(container.firstChild);
};

class StackManager {
    constructor() {
        this.stack = [];
        this.baseZIndex = 1000;
        
        // Single shared backdrop overlay element driven by boxes.css
        this.overlay = document.createElement("div");
        this.overlay.classList.add("modal-overlay-backdrop");
        this.overlay.style.display = "none";
        this.overlay.addEventListener("click", () => this.handleBackdropClick());
        document.body.appendChild(this.overlay);
    }

    push(dialog) {
        if (!this.stack.includes(dialog)) {
            this.stack.push(dialog);
            this.updateZIndices();
        }
    }

    remove(dialog) {
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
        const topModalIndex = this.stack.findLastIndex(d => d.isModal);

        if (topModalIndex !== -1) {
            const overlayZ = this.baseZIndex + (topModalIndex * 10);
            this.overlay.style.display = "block";
            this.overlay.style.zIndex = overlayZ;
        } else {
            this.overlay.style.display = "none";
        }

        // Adjust each dialog's z-index sequentially
        this.stack.forEach((dialog, index) => {
            const dialogZ = this.baseZIndex + (index * 10) + 5;
            dialog.container.style.zIndex = dialogZ;
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
    constructor({ 
        title = "", 
        content = [], 
        classes = [], 
        isModal = true, 
        closeOnOverlayClick = true,
        isTemporary = false 
    }) {
        this.isModal = isModal;
        this.closeOnOverlayClick = closeOnOverlayClick;
        this.isTemporary = isTemporary;
        this.contentDefinition = content;

        this.buildWindow(title, classes);
    }

    buildWindow(title, extraClasses) {
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
        this.container.style.display = "block";
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
 * Custom Promise-based dynamic form prompt modal
 */
const dynamicPrompt = ({ 
    title, 
    elements, 
    confirmText = "Save", 
    cancelText = "Cancel", 
    onConfirm, 
    onCancel, 
    onValidate = null 
}) => {
    return new Promise((resolve) => {
        let promptDialog;

        const clearErrors = (modalEl) => {
            modalEl.querySelectorAll('.modal-error-message').forEach(el => el.remove());
            modalEl.querySelectorAll('.invalid').forEach(el => el.classList.remove('invalid'));
        };

        const handleCancel = () => {
            promptDialog.close();
            if (onCancel) onCancel();
        };

        const handleConfirm = () => {
            const modalEl = promptDialog.container;
            clearErrors(modalEl);

            const data = {};
            modalEl.querySelectorAll('input[id], select[id], textarea[id]').forEach(el => {
                if (el.id) {
                    data[el.id] = el.type === 'number' ? el.valueAsNumber : el.value;
                }
            });

            if (onValidate) {
                const errors = onValidate(data);
                if (errors && Object.keys(errors).length > 0) {
                    Object.keys(errors).forEach(inputId => {
                        const inputEl = document.getElementById(inputId);
                        if (inputEl) {
                            inputEl.classList.add('invalid');
                            dynamicGenerator({
                                type: 'span',
                                classes: 'modal-error-message',
                                text: errors[inputId]
                            }, inputEl.parentNode);
                        }
                    });
                    return;
                }
            }

            if (onConfirm) onConfirm(data);
            resolve(data);
            promptDialog.close();
        };

        // Prepare elements target
        const contentElements = elements.map(el => ({
            ...el,
            target: el.target ?? '@modalContent'
        }));

        const promptDefinition = [
            { type: "div", classes: "modal-body-wrapper", varId: "modalContent" },
            { type: "div", classes: "modal-footer", varId: "modalFooter" },
            {
                type: "button",
                classes: ["btn", "btn-secondary"],
                text: cancelText,
                target: "@modalFooter",
                events: { click: handleCancel }
            },
            {
                type: "button",
                classes: ["btn", "btn-primary"],
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
            isModal: true,
            isTemporary: true
        });

        promptDialog.open();
    });
};

/**
 * HoverBox instance logic
 */
let boxes = {};

class Box extends BaseDialog {
    constructor(title, content, options = {}) {
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

    refreshContent(content) {
        cleanContainer(this.body);
        this.content = generateDynamic(content, this.body);
    }

    openBox() { this.open(); }
    closeBox() { this.close(); }
}

function addHoverBox(id, title, content, options = {}) {
    if (id === null) {
        if (options.isTemporary === undefined) options.isTemporary = true;
        let tempBox = new Box(title, content, options);
        return tempBox;
    } else {
        boxes[id] = new Box(title, content, options);
        return boxes[id];
    }
}