function resolveElement(elem){
    if(elem instanceof HTMLElement)
        return elem

    return document.querySelector(elem)
}

const dynamicGenerator = ( definition, location = document.body ) => {
    // Validate element type
    if (!definition.type || typeof definition.type !== "string") {
        console.error( "Invalid element type:", definition );
        return;
    }

    location = resolveElement(location)

    const validElementTypes = [ 'code', 'li', 'strong', 'small', 'pre', 'textarea', 'nav', 'section', 'a', 'i', 'div', 'span', 'p', 'br', 'button', 'h1', 'h2', 'h3', 'input', 'img', 'label', "table", "thead", "tbody", "select", "option", "tr", "td", "th" ];
    if (!validElementTypes.includes(definition.type)) {
        console.error("Unsupported element type:", definition.type);
        return;
    }

    const item = document.createElement(definition.type);

    if (definition.text !== undefined) {
        item.innerText = definition.text;
    } else if (definition.html !== undefined) {
        item.innerHTML = definition.html;
    }

    // Add classes if provided
    try{
        if (Array.isArray(definition.classes)) {
            definition.classes.filter( item => item.length !== 0 ).forEach(className => item.classList.add(className));
        } else if (typeof definition.classes === "string") {
            item.classList.add(definition.classes);
        }
    } catch( e ){
        console.error( definition );
        throw Error( e );
    }

    // Add Attributes
    if (typeof definition.attributes === "object") {
        for (const attribute in definition.attributes) item.setAttribute(attribute, definition.attributes[attribute]);
    }

    if( typeof definition.style === "object" )
        for( const style in definition.style ) item.style[ style ] = definition.style[ style ];

    if( typeof definition.properties === "object" )
        for( const property in definition.properties ) item[ property ] = definition.properties[ property ];

    // Set ID if provided
    if (definition.id !== undefined && typeof definition.id === "string") {
        item.id = definition.id
    }

    //data tags
    if( definition.data !== undefined && typeof definition.data === "object" ){
        Object.keys( definition.data ).forEach( tag => item.dataset[ tag ] = definition.data[ tag ] );
    }

    // Attach events if provided
    if( Array.isArray( definition.events ) ){
        definition.events.forEach( event => {
            if( event.type !== undefined && event.callback !== undefined ){
                item.addEventListener( event.type, event.callback.bind( this ) );
            }
        } );
    } else if( typeof definition.events == "object" ){
        for( const event in definition.events ) item.addEventListener( event, definition.events[ event ].bind( this ) );
    }

    if( definition.children !== undefined && Array.isArray( definition.children ) ){
        definition.children.forEach( child => dynamicGenerator( child, item ) );
    }
    
    location.append( item );
    return item;
}

const generateDynamic = ( definition, location = document.body ) => {
    if( !Array.isArray( definition ) ) return console.error( "Definition is not an array" );
    let ids = {};

    location = resolveElement(location);

    definition.forEach( item => {
        let target;
        if( item.target === undefined ){
            target = location;
        } else if( item.target.startsWith( "@" ) && ids[ item.target.slice( 1 ) ] ){
            target = ids[ item.target.slice( 1 ) ];
        } else if( item.target !== undefined && item.target.charAt[ 0 ] !== "@" ){
            target = document.querySelector( item.target );
        }
        if( !target ){
            console.error("Invalid target selector:", item.target, item);
            return;
        }
        const elem = dynamicGenerator( item, target );
        if( item.varId !== undefined ) ids[ item.varId ] = elem;
    } );
}

const dynamicPrompt = ( { title, elements, confirmText = "Save", cancelText = "Cancel", onConfirm, onCancel, onValidate = null } ) => {
    return new Promise((resolve, reject) => {
        const modalId = `modal-${ Date.now() }`;

        const closeModal = () => {
            const modal = document.getElementById( modalId );
            if( modal ) modal.remove();
            if( onCancel ) onCancel();
        };

        const clearErrors = modal => {
            modal.querySelectorAll( '.modal-error-message' ).forEach( el => el.remove() );
            modal.querySelectorAll( '.invalid' ).forEach( el => el.classList.remove( 'invalid' ) );
        };

        const confirmAction = () => {
            const modal = document.getElementById( modalId );
            if( !modal ) return;

            clearErrors( modal );

            const data = {};
            modal.querySelectorAll( 'input[id], select[id], textarea[id]' ).forEach( el => {
                if( el.id ){
                    const value = el.type === 'number' ? el.valueAsNumber : el.value;
                    data[ el.id ] = value;
                }
            } );

            if( onValidate ){
                const errors = onValidate( data );

                if( errors && Object.keys( errors ).length > 0 ){
                    Object.keys( errors ).forEach( inputId => {
                        const inputEl = document.getElementById( inputId );
                        if( inputEl ){
                            inputEl.classList.add( 'invalid' );
                            
                            const errorMsg = errors[ inputId ];
                            const errorElDef = {
                                type: 'span',
                                classes: 'modal-error-message',
                                text: errorMsg
                            };
                            dynamicGenerator( errorElDef, inputEl.parentNode );
                        }
                    } );
                    return;
                }
            }
            
            if( onConfirm ) onConfirm( data );
            resolve(data)
            closeModal();
        };

        const modalContentElements = elements.map( el => ({
            ...el,
            target: el.target ?? '@modalContent'
        } ) );

        const modalDefinition = [
            {
                type: "div",
                id: modalId,
                classes: "modal-overlay",
                varId: "overlay",
                events: { click: e => { if (e.target.id === modalId) closeModal(); } }
            },
            { type: "div", classes: "modal-box", varId: "modalBox", target: "@overlay" },
            
            { type: "div", classes: "modal-header", varId: "modalHeader", target: "@modalBox" },
            { type: "h2", text: title, target: "@modalHeader" },
            { 
                type: "button", 
                classes: "close-btn", 
                html: "&times;",
                target: "@modalHeader",
                events: { click: closeModal }
            },
            
            { type: "div", classes: "modal-content", varId: "modalContent", target: "@modalBox" },
            
            { type: "div", classes: "modal-footer", varId: "modalFooter", target: "@modalBox" },
            {
                type: "button",
                classes: ["btn", "btn-secondary"],
                text: cancelText,
                target: "@modalFooter",
                events: { click: closeModal }
            },
            {
                type: "button",
                classes: ["btn", "btn-primary"],
                text: confirmText,
                target: "@modalFooter",
                events: { click: confirmAction }
            },
            
            ...modalContentElements
        ];

        generateDynamic( modalDefinition );
    });
}

const cleanContainer = elem => {
    const container = resolveElement(elem);

    while ( container.firstChild ) {
        container.removeChild( container.firstChild );
    }
}