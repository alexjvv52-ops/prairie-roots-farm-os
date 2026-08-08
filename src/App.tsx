import { useDialogPointerGuard } from "@/lib/dialogPointerGuard";
import { Today } from "@/screens/Today";

function App() {
  useDialogPointerGuard();
  return <Today />;
}

export default App;
