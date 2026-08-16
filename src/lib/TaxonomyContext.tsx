import { createContext, useContext, useState, useEffect, useMemo, ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Subject } from "@/components/settings/TaxonomyManager";

// A backwards compatible shape for the old TOPICS_BY_SUBJECT format
export type TopicsBySubjectDict = Record<string, Record<string, string[]>>;

interface TaxonomyContextType {
  subjects: Subject[];
  topicsBySubject: TopicsBySubjectDict;
  loading: boolean;
}

const TaxonomyContext = createContext<TaxonomyContextType | undefined>(undefined);

export function TaxonomyProvider({ children }: { children: ReactNode }) {
  const [subjects, setSubjects] = useState<Subject[]>([]);
  const [loading, setLoading] = useState(true);

  const fetchTaxonomy = async () => {
    try {
      const data = await invoke<Subject[]>("get_taxonomy_tree");
      setSubjects(Array.isArray(data) ? data : []);
    } catch (e) {
      console.error("Failed to load taxonomy tree:", e);
      setSubjects([]);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    fetchTaxonomy();

    const unlistenPromise = listen("taxonomy-changed", () => {
      fetchTaxonomy();
    });

    return () => {
      unlistenPromise.then((unlisten) => unlisten());
    };
  }, []);

  // Compute the legacy TOPICS_BY_SUBJECT dictionary dynamically with strict null-safety
  const topicsBySubject = useMemo<TopicsBySubjectDict>(() => {
    const dict: TopicsBySubjectDict = {};
    if (!Array.isArray(subjects)) return dict;

    for (const subject of subjects) {
      if (!subject || !subject.name) continue;
      dict[subject.name] = {};

      if (Array.isArray(subject.modules)) {
        for (const module of subject.modules) {
          if (!module || !module.name) continue;
          dict[subject.name][module.name] = Array.isArray(module.topics)
            ? module.topics
                .map((t: any) => (typeof t === "string" ? t : t?.name))
                .filter((n: any): n is string => typeof n === "string" && n.trim().length > 0)
            : [];
        }
      }
    }
    return dict;
  }, [subjects]);

  return (
    <TaxonomyContext.Provider value={{ subjects: subjects || [], topicsBySubject, loading }}>
      {children}
    </TaxonomyContext.Provider>
  );
}

export function useTaxonomy() {
  const context = useContext(TaxonomyContext);
  if (context === undefined) {
    throw new Error("useTaxonomy must be used within a TaxonomyProvider");
  }
  return context;
}
