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
        return boxes[ id ];
    }
}

class box {
    constructor( title, content, options ){
        this.options = options;
        this.options.isTemporary = this.options.isTemporary || false;
        this.createBox( title, content );
        this.setStyle();
        this.content = generateDynamic(content, this.boxBody);
        if( this.options.isTemporary ) this.openBox();
    }

    createBox( title ){
        const items = generateDynamic([
            {type: "div", classes: "boxContainer", varId: "container"},
            {type: "div", classes: "boxTitle", target: "@container", varId: "boxTitle"},
            {type: "span", text: title, target: "@boxTitle"},
            {type: "svg", file: "icons/close.svg", classes: "closeBtn", events: {click: this.closeBox.bind(this)}, target: "@boxTitle", varId: "closeBtn"},
            {type: "div", classes: "boxBody", target: "@container", varId: "body"}
        ]);

        this.boxContainer = items.container;
        this.boxBody = items.body;
    }

    refreshContent( content ){
        while ( this.boxBody.firstChild ) {
            this.boxBody.removeChild( this.boxBody.firstChild );
        }
        this.content = {};
        content.forEach( element => this.addElement( element ) );
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