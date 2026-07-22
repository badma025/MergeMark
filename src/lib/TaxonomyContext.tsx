import { createContext, useContext, useState, useEffect, ReactNode } from "react";
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
      setSubjects(data);
    } catch (e) {
      console.error("Failed to load taxonomy tree:", e);
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

  // Compute the legacy TOPICS_BY_SUBJECT dictionary dynamically
  const topicsBySubject: TopicsBySubjectDict = {};
  for (const subject of subjects) {
    topicsBySubject[subject.name] = {};
    for (const module of subject.modules) {
      topicsBySubject[subject.name][module.name] = module.topics.map((t: any) => t.name);
    }
  }

  return (
    <TaxonomyContext.Provider value={{ subjects, topicsBySubject, loading }}>
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
