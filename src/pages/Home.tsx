import { immer } from "zustand/middleware/immer";
import { create } from "zustand/react";

interface PageState {
  count: number;
  setCount(count: number): void;
}

const usePageState = create<PageState>()(
  immer((set) => ({
    count: 0,
    setCount(count) {
      set((state) => {
        state.count = count;
      });
    },
  })),
);

export default function HomePage() {
  const { count, setCount } = usePageState();

  return <div onClick={() => setCount(count + 1)}>{count}</div>;
}
