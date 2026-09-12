import { useForm } from "@tanstack/react-form";

import { Button } from "@/components/ui/button";
import env from "@/lib/env";

import { greet } from "../wasm/aegis_wasm";

export default function HomePage() {
  const form = useForm({});

  return (
    <div>
      <p onClick={() => greet("hello")}>{env.VITE_BACKEND_URL}</p>
      <Button>Hello</Button>
    </div>
  );
}
