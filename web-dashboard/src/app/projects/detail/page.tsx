"use client";

import { Suspense } from "react";
import { useSearchParams } from "next/navigation";
import ProjectDetail from "./project-detail-client";

function DetailContent() {
  const searchParams = useSearchParams();
  const id = searchParams.get("id") || "";
  return <ProjectDetail id={id} />;
}

export default function ProjectDetailPage() {
  return (
    <Suspense fallback={<div className="p-8 text-muted-foreground">Loading project...</div>}>
      <DetailContent />
    </Suspense>
  );
}
