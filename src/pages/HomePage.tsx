import { useDeleteUser, useGetUser, usePostUser } from "../api";
import { greet } from "../wasm/aegis_wasm";

export default function HomePage() {
  const query = useGetUser();
  const mutation = usePostUser();
  const mutationDelete = useDeleteUser();

  return <p onClick={() => greet("hello")}>{JSON.stringify(import.meta.env.VITE_BACKEND_URL)}</p>;
}
