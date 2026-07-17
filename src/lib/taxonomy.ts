export const SUBJECTS = [
  "A Level Mathematics (Edexcel)",
  "A Level Further Mathematics (Edexcel)",
  "GCSE Mathematics (Edexcel)",
  "GCSE Further Mathematics (AQA)"
];

export const TOPICS_BY_SUBJECT: Record<string, Record<string, string[]>> = {
  "A Level Mathematics (Edexcel)": {
    "Pure": [
      "Proof", "Algebra and functions", "Coordinate geometry in the (x, y) plane", 
      "Sequences and series", "Trigonometry", "Exponentials and logarithms", 
      "Differentiation", "Integration", "Numerical methods", "Vectors"
    ],
    "Statistics": [
      "Statistical sampling", "Data presentation and interpretation", "Probability", 
      "Statistical distributions", "Statistical hypothesis testing"
    ],
    "Mechanics": [
      "Quantities and units in mechanics", "Kinematics", "Forces and Newton's laws", "Moments"
    ]
  },
  "A Level Further Mathematics (Edexcel)": {
    "Core Pure": [
      "Complex numbers", "Argand diagrams", "Series", "Roots of polynomials", 
      "Volumes of revolution", "Matrices", "Linear transformations", 
      "Proof by induction", "Vectors", "Differential equations", 
      "Polar coordinates", "Hyperbolic functions", "Maclaurin series", 
      "Methods in calculus"
    ],
    "Further Mechanics 1": [
      "Momentum and impulse", "Work, energy and power", "Elastic strings and springs", 
      "Elastic collisions in one dimension", "Elastic collisions in two dimensions"
    ],
    "Further Statistics 1": [
      "Discrete probability distributions", "Poisson distribution", 
      "Geometric and negative binomial", "Hypothesis testing", 
      "Central Limit Theorem", "Chi-squared tests", "Probability generating functions", 
      "Quality of tests"
    ],
    "Further Pure 1": [
      "Vectors (Cross product & planes)", "Conic sections", "Inequalities", 
      "t-formulae", "Taylor series", "Numerical methods (Further)", 
      "Reducible differential equations"
    ],
    "Decision Mathematics 1": [
      "Algorithms", "Graphs and networks", "Algorithms on graphs", 
      "Route inspection", "Travelling Salesperson Problem", 
      "Linear programming", "Simplex algorithm"
    ],
    "Further Pure 2": [
      "Number theory", "Groups", "Further calculus", "Further matrix algebra", 
      "Further complex numbers", "Maclaurin series"
    ],
    "Further Mechanics 2": [
      "Circular motion", "Centres of mass of plane figures", "Further centres of mass", 
      "Kinematics", "Dynamics"
    ],
    "Further Statistics 2": [
      "Linear regression", "Continuous probability distributions", 
      "Correlation", "Hypothesis testing"
    ],
    "Decision Mathematics 2": [
      "Transportation problems", "Allocation (assignment) problems", "Flows in networks", 
      "Dynamic programming", "Game theory", "Recurrence relations", "Decision analysis"
    ]
  },
  "GCSE Mathematics (Edexcel)": {
    "GCSE Mathematics": [
      "Number", "Algebra", "Ratio, proportion and rates of change", 
      "Geometry and measures", "Probability", "Statistics"
    ]
  },
  "GCSE Further Mathematics (AQA)": {
    "GCSE Further Mathematics": [
      "Number", "Algebra", "Coordinate Geometry", "Calculus", 
      "Matrix Transformations", "Geometry"
    ]
  },
  /*"Physics": {
    "Physics": [
      "Measurements and their errors", "Particles and radiation", 
      "Waves", "Mechanics and materials", "Electricity", 
      "Further mechanics", "Thermal physics", 
      "Fields and their consequences", "Nuclear physics",
      "Telescopes", "Classification of stars", "Cosmology"
    ]
  },
  "Computer Science": {
    "Computer Science": [
      "Fundamentals of programming", "Fundamentals of data structures", 
      "Fundamentals of algorithms", "Theory of computation", 
      "Fundamentals of data representation", "Fundamentals of computer systems", 
      "Computer organisation and architecture", "Consequences of uses of computing", 
      "Communication and networking", "Fundamentals of databases", 
      "Big Data", "Fundamentals of functional programming"
    ]
  }*/
};

export const ALL_TOPICS = Array.from(new Set(
  Object.values(TOPICS_BY_SUBJECT)
    .flatMap(subjectMods => Object.values(subjectMods).flat())
));
