import { greet } from "../wasm/aegis_wasm";

export default function HomePage() {
  return <p onClick={() => greet("hello")}>Home</p>;
}
