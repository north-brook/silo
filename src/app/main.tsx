import { createRoot } from "react-dom/client";
import { BrowserRouter } from "react-router-dom";
import { AppProviders } from "./providers";
import { AppRouter } from "./router";
import "./styles.css";

const container = document.getElementById("root");

if (!container) {
	throw new Error("Missing #root container");
}

createRoot(container).render(
	<BrowserRouter>
		<AppProviders>
			<AppRouter />
		</AppProviders>
	</BrowserRouter>,
);
