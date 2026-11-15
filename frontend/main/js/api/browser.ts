const randomKeyChars = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
let openRequests: Record<string, {resolve: Function, reject: Function}> = {};

function generateRequestKey (length: number, group: object | null): string {
    if(!group) return Array.from({length: length}, () => randomKeyChars[Math.floor(Math.random() * randomKeyChars.length)]).join("");
    let key: string;
        do{
            key = Array.from({length: length}, () => randomKeyChars[Math.floor(Math.random() * randomKeyChars.length)]).join("");
        } while(group.hasOwnProperty(key));
    return key;
}

const api = {
    getResponse: async (endpoint: string, data: any): Promise<any> => {
        const request = await fetch(`/api/${endpoint}`, {
            method: "POST",
            headers: {
                "Content-Type": "application/json"
            },
            body: JSON.stringify(data)
        });
        return await request.json();
    }
};

export { api };