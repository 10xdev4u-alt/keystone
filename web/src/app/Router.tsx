import { createBrowserRouter, RouterProvider } from "react-router-dom";
import { routesConfig } from "./routes";
import "./routes.css";

/** Builds the singleton browser router from the route map. */
const router = createBrowserRouter(routesConfig);

export function Router() {
  return <RouterProvider router={router} />;
}
