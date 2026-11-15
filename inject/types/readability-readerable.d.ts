// The probe imports only the readerable check (a CommonJS module whose
// export is the function), so its bundle stays small.
declare module "@mozilla/readability/Readability-readerable.js" {
  import type { isProbablyReaderable } from "@mozilla/readability";

  const check: typeof isProbablyReaderable;
  export default check;
}
