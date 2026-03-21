const { createUser, isAdmin, genId } = require("./helpers");

/**
 * Create a project owned by a user.
 * @param {string} title
 * @param {Object} owner - Must have an `id` field
 * @returns {Object}
 */
function createProject(title, owner) {
  return {
    id: genId("p"),
    title,
    ownerId: owner.id,
    tags: [],
  };
}

/**
 * Run the demo workflow.
 */
function main() {
  const alice = createUser("Alice", "alice@example.com", "admin");
  const bob = createUser("Bob", "bob@example.com");

  const project = createProject("supp demo", alice);

  console.log(`Project: ${project.title} (owner: ${alice.name})`);
  console.log(`Alice is admin: ${isAdmin(alice)}`);
  console.log(`Bob is admin: ${isAdmin(bob)}`);
}

main();
