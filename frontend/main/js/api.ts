const isApp = JSON.parse(window.localStorage.getItem("isApp") || "false");

const { api } = await (
    isApp
        ? import("./api/app")
        : import("./api/browser")
);

export { api };