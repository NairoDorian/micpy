import ReactDOM from "react-dom/client";
import App from "./App";
import "./index.css";

/** Application entry point — mounts the <App /> root component into #root. */
ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <App />,
);
