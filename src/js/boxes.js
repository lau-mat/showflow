const modal = document.createElement( "div" );
modal.classList.add( "modal" );
document.body.append( modal );


let boxes = {};

function addHoverBox( id, title, content, options = {} ){
    if( id === null ){
        if( options.isTemporary == undefined ) options.isTemporary = true;
        let tempBox = new box( title, content, options );
        tempBox.openBox();
    } else {
        boxes[ id ] = new box( title, content, options );
    }
}

class box {
    constructor( title, content, options ){
        this.options = options;
        this.options.isTemporary = this.options.isTemporary || false;
        this.content = {};
        this.createBox( title, content );
        this.setStyle();
        content.forEach( element => this.addElement( element ) );
        if( this.options.isTemporary ) this.openBox();
    }

    createBox( title ){
        this.boxContainer = document.createElement( "div" );
        this.boxContainer.classList.add( "boxContainer" );

        const boxTitle = document.createElement( "div" );
        boxTitle.classList.add( "boxTitle" );
        boxTitle.innerText = title;

        const closeBtn = document.createElement( "img" );
        closeBtn.src = "icon/close.png";
        closeBtn.classList.add( "closeBtn" );
        closeBtn.addEventListener( "click", this.closeBox.bind( this ) );

        boxTitle.append( closeBtn );
        this.boxContainer.append( boxTitle );

        this.boxBody = document.createElement( "div" );
        this.boxBody.classList.add( "boxBody" );

        this.boxContainer.append( this.boxBody );
        document.body.append( this.boxContainer );
    }

    refreshContent( content ){
        while ( this.boxBody.firstChild ) {
            this.boxBody.removeChild( this.boxBody.firstChild );
        }
        this.content = {};
        content.forEach( element => this.addElement( element ) );
    }

    addElement( definition ){
        // Validate element type
        if( !definition.type || typeof definition.type !== "string" ){
            console.error( "Invalid element type:", definition.type );
            return;
        }

        const validElementTypes = [ 'a', 'h3', 'div', 'span', 'p', 'br', 'button', 'h1', 'h2', 'input', 'textarea', 'img', 'label', "select", "option", "table", "thead", "tbody", "td", "tr", "th" ];
        if( !validElementTypes.includes( definition.type ) ){
            console.error( "Unsupported element type:", definition.type );
            return;
        }

        let target;
        if( definition.target === undefined ){
            target = this.boxBody;
        } else if( definition.target.startsWith( "@" ) && this.content[ definition.target.slice( 1 ) ] ){
            target = this.content[ definition.target.slice( 1 ) ];
        } else if( definition.target !== undefined && definition.target.charAt[ 0 ] !== "@" ){
            target = this.boxContainer.querySelector( definition.target );
        }
        if( !target ){
            console.error("Invalid target selector:", definition.target);
            return;
        }

        const item = document.createElement( definition.type );

        if( definition.text !== undefined ){
            item.innerText = definition.text;
        } else if( definition.html !== undefined ){
            item.innerHTML = definition.html;
        }

        // Add classes if provided
        if( Array.isArray( definition.classes ) ){
            definition.classes.forEach( className => item.classList.add( className ) );
        } else if( typeof definition.classes === "string" ){
            item.classList.add( definition.classes );
        }

        // Add Attributes
        if( typeof definition.attributes === "object" ){
            for( const attribute in definition.attributes ) {
                item.setAttribute(  attribute, definition.attributes[ attribute ] );
            }
        }

        //data tags
        if( typeof definition.data === "object" ){
            for( const data in definition.data ){
                item.dataset[ data ] = definition.data[ data ];
            }
        }

        // Set ID if provided
        if( definition.id !== undefined && typeof definition.id === "string" ){
            item.id = definition.id;
            this.content[ definition.id ] = item;
        } else if( definition.varId !== undefined ){
            this.content[ definition.varId ] = item;
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

        // Append to the target
        target.append( item );
    }

    setStyle(){
        this.options.width = this.options.width || "75%";
        this.options.height = this.options.height || "50%";
        this.options.modal = this.options.modal || true;
        this.options.fullscreen = this.options.fullscreen || false;

        if( this.options.fullscreen ) {
            this.boxContainer.style.width = "100%";
            this.boxContainer.style.height = "100%";
            this.boxContainer.style.position = "fixed";
            this.boxContainer.style.top = 0;
            this.boxContainer.style.left = 0;
            this.boxContainer.style.right = 0;
            this.boxContainer.style.transform = "none";
            this.boxContainer.style.zIndex = 200;
        } else {
            this.boxContainer.style.width = this.options.width;
            this.boxContainer.style.height = this.options.height;
        }
    }

    openBox(){
        this.boxContainer.style.display = "block";
        if( this.options.modal ) modal.style.display = "block";
        document.body.style.overflow = "hidden";
    }

    closeBox(){
        this.boxContainer.style.display = "none";
        if( this.options.modal ) modal.style.display = "none";
        if( this.options.isTemporary ) this.boxContainer.remove();
        document.body.style.overflow = "";
    }
}