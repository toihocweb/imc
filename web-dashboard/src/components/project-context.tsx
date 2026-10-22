"use client";

import React, { createContext, useContext, useEffect, useState } from "react";

export interface Project {
  id: string;
  name: string;
  path: string;
  counts: {
    files: number;
    symbols: number;
    edges: number;
    chunks: number;
    embeddings: number;
  };
  db_size_mb: number;
  indexed_at?: string;
}

interface ProjectContextType {
  projects: Project[];
  selectedProject: Project | null;
  selectedPath: string;
  setSelectedPath: (path: string) => void;
  refreshProjects: () => Promise<void>;
  loading: boolean;
}

const ProjectContext = createContext<ProjectContextType>({
  projects: [],
  selectedProject: null,
  selectedPath: "",
  setSelectedPath: () => {},
  refreshProjects: async () => {},
  loading: true,
});

export function ProjectProvider({ children }: { children: React.ReactNode }) {
  const [projects, setProjects] = useState<Project[]>([]);
  const [selectedPath, setSelectedPathState] = useState<string>("");
  const [loading, setLoading] = useState(true);

  const setSelectedPath = (path: string) => {
    setSelectedPathState(path);
    if (typeof window !== "undefined") {
      try {
        localStorage.setItem("imc_selected_project_path", path);
      } catch {}
    }
  };

  const refreshProjects = async () => {
    try {
      const res = await fetch("/api/projects");
      const data = await res.json();
      if (Array.isArray(data)) {
        setProjects(data);
        if (data.length > 0) {
          const saved = typeof window !== "undefined" ? localStorage.getItem("imc_selected_project_path") : null;
          const match = data.find((p) => p.path === saved);
          if (match) {
            setSelectedPathState(match.path);
          } else if (!selectedPath || !data.some((p) => p.path === selectedPath)) {
            setSelectedPathState(data[0].path);
          }
        }
      }
    } catch (err) {
      console.error("Failed to load projects", err);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    refreshProjects();
  }, []);

  const selectedProject = projects.find((p) => p.path === selectedPath) || (projects.length > 0 ? projects[0] : null);

  return (
    <ProjectContext.Provider
      value={{
        projects,
        selectedProject,
        selectedPath: selectedProject ? selectedProject.path : "",
        setSelectedPath,
        refreshProjects,
        loading,
      }}
    >
      {children}
    </ProjectContext.Provider>
  );
}

export function useProject() {
  return useContext(ProjectContext);
}
