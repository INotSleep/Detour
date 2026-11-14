import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import { Crash, catchWindowErrors } from "./components/Crash";
import "./styles.css";

catchWindowErrors();

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Crash>
      <App />
    </Crash>
  </StrictMode>,
);
