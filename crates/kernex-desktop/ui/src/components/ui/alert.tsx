import * as React from "react";
import { cn } from "@/lib/utils";

export const Alert = React.forwardRef<HTMLDivElement, React.HTMLAttributes<HTMLDivElement>>(({ className, ...props }, ref) => <div ref={ref} role="alert" className={cn("relative w-full rounded-lg border p-4 text-sm", className)} {...props} />);
Alert.displayName = "Alert";
export const AlertTitle = ({ className, ...props }: React.HTMLAttributes<HTMLHeadingElement>) => <h5 className={cn("mb-1 font-medium", className)} {...props} />;
export const AlertDescription = ({ className, ...props }: React.HTMLAttributes<HTMLDivElement>) => <div className={cn("text-sm text-muted-foreground", className)} {...props} />;
