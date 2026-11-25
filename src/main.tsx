import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./styles.css";

// No browser context menu / reload shortcuts in a desktop app.
document.addEventListener("contextmenu", (e) => e.preventDefault());

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
