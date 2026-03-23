import { useChromeStorage } from "@runesh/ui/hooks/use-chrome-storage";

export function App() {
  const [count, setCount] = useChromeStorage("popup_count", 0);

  return (
    <div className="w-80 p-4 space-y-4">
      <h1 className="text-lg font-bold">YOUR_APP</h1>
      <p className="text-sm text-muted-foreground">
        Chrome Extension
      </p>
      <button
        className="inline-flex items-center justify-center rounded-md bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90"
        onClick={() => setCount((c) => c + 1)}
      >
        Count: {count}
      </button>
    </div>
  );
}
