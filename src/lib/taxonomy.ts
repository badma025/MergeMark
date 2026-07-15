export const SUBJECTS = [
  "Mathematics",
  "Further Mathematics",
  "Physics",
  "Computer Science"
];

export const TOPICS_BY_SUBJECT: Record<string, string[]> = {
  "Mathematics": [
    "Proof", "Algebra and functions", "Coordinate geometry in the (x, y) plane", 
    "Sequences and series", "Trigonometry", "Exponentials and logarithms", 
    "Differentiation", "Integration", "Numerical methods", "Vectors",
    "Statistical sampling", "Data presentation and interpretation", "Probability", 
    "Statistical distributions", "Statistical hypothesis testing",
    "Quantities and units in mechanics", "Kinematics", "Forces and Newton's laws", "Moments"
  ],
  "Further Mathematics": [
    "Complex numbers", "Argand diagrams", "Series", "Roots of polynomials", 
    "Volumes of revolution", "Matrices", "Linear transformations", 
    "Proof by induction", "Vectors", "Differential equations", 
    "Polar coordinates", "Hyperbolic functions", "Maclaurin series", 
    "Methods in calculus", "Momentum and impulse", "Work, energy and power", 
    "Elastic strings and springs", "Elastic collisions in one dimension", 
    "Elastic collisions in two dimensions", "Discrete probability distributions", 
    "Poisson distribution", "Geometric and negative binomial", "Hypothesis testing", 
    "Central Limit Theorem", "Chi-squared tests", "Probability generating functions", 
    "Quality of tests", "Vectors (Cross product & planes)", "Conic sections", 
    "Inequalities", "t-formulae", "Taylor series", "Numerical methods (Further)", 
    "Reducible differential equations", "Algorithms", "Graphs and networks", 
    "Algorithms on graphs", "Route inspection", "Travelling Salesperson Problem", 
    "Linear programming", "Simplex algorithm"
  ],
  "Physics": [
    "Measurements and their errors", "Particles and radiation", 
    "Waves", "Mechanics and materials", "Electricity", 
    "Further mechanics", "Thermal physics", 
    "Fields and their consequences", "Nuclear physics",
    "Telescopes", "Classification of stars", "Cosmology"
  ],
  "Computer Science": [
    "Fundamentals of programming", "Fundamentals of data structures", 
    "Fundamentals of algorithms", "Theory of computation", 
    "Fundamentals of data representation", "Fundamentals of computer systems", 
    "Computer organisation and architecture", "Consequences of uses of computing", 
    "Communication and networking", "Fundamentals of databases", 
    "Big Data", "Fundamentals of functional programming"
  ]
};

export const ALL_TOPICS = Array.from(new Set(Object.values(TOPICS_BY_SUBJECT).flat()));
